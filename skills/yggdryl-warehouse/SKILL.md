---
name: yggdryl-warehouse
description: Find and open tables by dotted path through yggdryl's warehouse - catalogs of namespaces of tables - in Rust, Python and Node.js. Use when registering a folder, an object-store prefix or a ZIP archive as a catalog (FolderCatalog, Catalog.folder, Catalog.from_url), registering one table at any location under a path (MediaTable, Table.media), resolving `lake.eu.trades` to a handle (Warehouse.table, SystemWarehouse.table), listing one level (namespaces / tables views, children), checking membership, writing through a view (tables.append / overwrite), stating credentials once as properties that reach every table's storage, reading an object's kind, path, storage and schema, or holding a catalog, namespace or table as an IOBase (Holder::Catalog / Namespace / Table, IOBase.from). Covers the memory and folder implementations and the process's system warehouse.
---

# Warehouse: catalogs, namespaces, tables

One abstraction for every place that answers "which tables are there, and how
do I read one". Four traits say what every object a path reaches answers
(`ObjectValue`, `NamespaceValue`, `CatalogValue`, `TableValue`) and four enums
say which implementation answers it (`Object`, `Catalog`, `Namespace`,
`Table`), the way `Holder` does for storage. An object is a **description** -
its path, what it states, where its storage is - never a view borrowed from
its parent, so it sits in an enum, registers in a `Warehouse`, crosses a
binding, and is also a **handle**: a catalog or a namespace a container whose
`ls` yields its children, a table the rows every record verb reaches.

Bytes, listings and backends of a handle are `yggdryl-storage`; rows on a
handle, `RecordOptions` and partitions are `yggdryl-records`; a plan's
`from catalog.namespace.table` is `yggdryl-expressions`. Install and
cross-language conventions are in `yggdryl`.

## Choose the door

| Task | Rust | Python | JavaScript |
| --- | --- | --- | --- |
| a folder as a catalog, a handle in hand | `FolderCatalog::bound(name, holder)` | `FolderCatalog(name, IOBase(p))` | `warehouse.Catalog.folder(name, new IOBase(p))` |
| a folder as a catalog, by location | `FolderCatalog::new(name, url)?` (`.with_levels(n)`, `.with_properties(bag)`) | `FolderCatalog(name, path_or_url, levels=1, **properties)` | `warehouse.Catalog.folder(name, location, { levels, properties })`, `new warehouse.FolderCatalog(...)` |
| a catalog from a URL and a property bag | `Catalog::from_url(&url, &properties)?` | `Catalog.from_url(url, type="memory", name="m")` | `warehouse.Catalog.fromUrl(url, { type, name })` |
| an Iceberg warehouse folder as a catalog - the one implementation that creates | `IcebergCatalog::bound(name, holder)`, `::new(name, url)?`, `Catalog::from_url` with `type = hadoop` (`iceberg` feature) | `IcebergCatalog(name, location, **properties)` from `yggdryl.iceberg` | `new iceberg.IcebergCatalog(name, location, { description, properties })`, `.intoCatalog()` to register |
| a namespace of registered objects | `MemoryNamespace::new("lake.eu")?.with_object(t)?` | `MemoryNamespace("lake.eu", objects=[t])` | `warehouse.Namespace.memory('lake.eu', { objects: [t] })` |
| a folder as a standalone namespace | `FolderNamespace::new("lake.eu", url)?`, `::bound(path, holder)?` | `FolderNamespace("lake.eu", location, levels=0)` | `warehouse.Namespace.folder('lake.eu', location)` |
| a table at any location | `MediaTable::new("lake.eu.trades", url)?` (`.with_field`, `.with_dtype`, `.with_layout`) | `MediaTable(path, location, field=, dtype=, layout="leaf", **properties)` | `warehouse.Table.media(path, location, { field, dtype, layout, properties })` |
| a registry | `Warehouse::new()`, `register(obj)?`, `replace(obj)?`, `unregister(path)?` | `Warehouse()`, `.register(obj)`, `.replace`, `.unregister` | `new warehouse.Warehouse()`, `.register`, `.replace`, `.unregister` |
| the process's registry | `SystemWarehouse::register(obj)?`, `::table(path)?` | `SystemWarehouse.register(obj)`, `.table(path)` | `warehouse.SystemWarehouse.register(obj)`, `.table(path)` |
| a plan over a registered table | `plan.execute()?` (the process's registry), `plan.execute_in(&warehouse)?`; `from lake.eu.trades` is the table at that path | `Plan(text).execute()`, `.execute_in(warehouse)` | `new Plan(text).execute()`, `.executeIn(registry)` |
| resolve a path | `warehouse.table(path)?`, `.namespace(path)?`, `.get(path)?` -> `Object`; `catalog.resolve(path)?` | `warehouse.table(path)`, `.namespace`, `.get`; `catalog.resolve(path)` | `registry.table(path)`, `.namespace`, `.get`; `catalog.resolve(path)` |
| a path | `"lake.\"eu west\".fills"`, `["lake", "eu west", "fills"]` | `'lake."eu west".fills'`, `["lake", "eu west", "fills"]` | `'lake."eu west".fills'`, `['lake', 'eu west', 'fills']` |
| one level | `catalog.namespaces()`, `.tables()`, `.children()` | `catalog.namespaces` (mapping), `.tables`, `.children()` | `catalog.namespaces()` (Map-like), `.tables()`, `.children()` |
| names, membership, count | `tables.iter()` -> `Names`, `contains(name)?`, `len()?`, `is_empty()?` | `list(tables)`, `name in tables`, `len(tables)` | `[...tables.keys()]`, `tables.has(name)`, `tables.size` |
| open one | `tables.get("eu.fills")?` | `tables["eu.fills"]` (`KeyError` when absent), `tables.get(name, default)` | `tables.get('eu.fills')`, `tables.get(['eu west', 'fills'])` |
| open or create | `tables.open_or_create(name, &field, &props)?` | `tables.open_or_create(name, field, **props)` | `tables.openOrCreate(name, field, props)` |
| write through the view | `tables.append_arrow_reader(name, reader)?`, `overwrite_arrow_reader`, `*_with_options` | `tables.append(name, data, options=, **props)`, `overwrite` | `tables.append(name, data, options?)`, `overwrite` |
| the rows | `table.read_arrow(None)?`, `read_arrow_reader(&options)?`, `row_size()?` (`IOMedia`) | `table.read_arrow_reader().read_all()`, `read_records()`, `row_size()` | `IOBase.from(table).readArrowReader().intoTable()`, `.rowSize()` |
| the schema, no row read | `table.field()?` (`TableValue`) | `table.field()` | `table.field()` |
| what holds the rows | `table.storage()` -> media type, `directory`, `table` | `table.storage` | `table.storage` |
| identity | `name()`, `path()`, `kind()` -> `IOKind`, `to_string()` dotted (`ObjectValue`) | `name`, `path` (tuple), `kind()` -> `"catalog"`, `str(obj)` | `name`, `path`, `kind` -> `'catalog'`, `String(obj)`, `implementation` |
| properties | `obj.properties()?` -> `Properties`; `Properties::new().with_property(k, v)` | `obj.properties` -> `dict[str, str]`; `properties={...}` or `**kw` | `obj.properties` -> ordered object; `{ properties: {...} }` |
| properties of a location | `warehouse.properties_for(&url)` | `warehouse.properties_for(url_or_path)` | `registry.propertiesFor(url)` |
| an object as a handle | `Holder::from(obj)`, `obj.into_holder()` | every object is an `IOBase` | `IOBase.from(obj)`, `new IOBase(obj)` |

## Rules for fast, correct use

1. **Construction touches nothing; the handle opens on the first verb.** A
   folder is listed when it is asked and nothing is cached but the container
   handle, so a table written a moment ago is found on the next ask. Hold
   the object, not a listing.
2. **A path costs one listing per level; a registered path costs none.**
   `warehouse.table("market.eu.trades")` over a folder catalog is one
   listing of the root, one of `eu` (plus one of `metadata/` per folder it
   passes, the one question that tells a table format from a plain folder),
   and no read. Resolving a registered `MediaTable` touches no store at all.
   `len()`/`size` drain a listing - ask `is_empty()`/`contains` instead when
   that is the question.
3. **Paths go through the grammar, never `split('.')`.** Dotted text is read
   by the plan's location grammar: quote a part with `"..."`, backticks or
   `[...]` (`lake."eu west".fills`), or pass the parts as a list and they
   arrive as they are, dot and all. The empty text is the root. A URL or a
   `with (...)` clause where a path is expected is refused at `$.path`.
4. **Properties inherit downward and open every handle.** An object's
   effective bag is its parent's, then what its store keeps, then what it
   states, a nearer statement winning by name. State credentials once on the
   catalog and every table's storage under it opens with them; a `codec` or
   `media_type` stated on a table types an extensionless location.
   `update_properties` persists only where the store keeps something - the
   memory, folder and media implementations keep nothing and refuse by name;
   an Iceberg catalog and namespace keep theirs in their own document.
5. **Memory lists what was registered; a folder lists its store; neither
   creates.** `create`, `open_or_create` and the append/overwrite helpers
   refuse a name nothing holds by implementation name
   (`filesystem "FolderCatalog" does not support creating a table`). Write to
   an existing table, or register a `MediaTable` at the path first and write
   through it - the handle creates the leaf. An Iceberg catalog creates,
   through existing namespaces only: make `sales` before `sales.orders`, or
   `tables.create("sales.orders", ..)` is the absence of `sales`.
6. **Registration builds memory levels only.** A table registered at
   `lake.eu.trades` creates the memory catalog `lake` and namespace `eu` as
   needed; a path under a folder catalog, a folder namespace or a table is
   refused (`does not support registering under an object that lists its own
   store`). A taken name conflicts; `replace` swaps and answers the old
   object; `unregister` removes and answers it.
7. **Folder rules.** One namespace level by default (`catalog.schema.table`;
   `levels` changes it, zero under a standalone namespace): a folder within
   the levels is a namespace, deeper a table of the rows beneath it
   (`storage == "directory"`); a folder whose store says `IOKind::Table` or
   whose `metadata/` holds a version hint or a metadata document is a table
   format at any depth (`storage == "table"`, rows need the `iceberg`
   feature, which answers it as `Table::Iceberg` - Python `IcebergTable`); a leaf is a table when a record medium reads its name's media
   type, named less every extension a media type claims (`trades.arrows.gz`
   is `trades`); a dot-prefixed entry is private; two entries of one name are
   both listed and asking for the name is a conflict.
8. **An object is a handle, and its kind says what it answers.** A catalog or
   a namespace is a container: `ls`, `child_by_path("a/b")`, `is_dir`; its
   byte verbs are `NotAtomic` and its record verbs name a table under it. A
   table is its rows: every record verb; its `kind()` is its storage's
   (`file` for a leaf, `directory`, `table`) and `is_tabular()` is true.
9. **Absence names the path; a kind mismatch is absence.**
   `expected a table at "lake.eu.trades", got nothing`; a namespace where a
   table is asked for, or a table met before the last part, is absence too.
   A memory level names an `object`, a folder level a `table`.
10. **The system warehouse is process-global under a lock.** It starts with
    the memory catalog `local` (`temporary`, `home`, `config`); register
    names unique to the process and unregister them in tests.
11. **A clone starts unresolved.** It rebuilds its handle from its location
    on its first verb (one role resolution more than the handle in hand); an
    object bound to a handle with no location - an in-memory `Buffer` -
    cannot be rebuilt and says so by name after a clone.
12. **Rust: bring the trait in scope, and name it when two define a verb.**
    `ObjectValue` for `name`/`path`/`kind`/`properties`, `NamespaceValue` for
    `children`/`get`, `CatalogValue` for `namespace_levels`, `TableValue` for
    `field`/`storage`, `IOMedia` for `row_size`/`read_arrow`. `IOBase` and
    `ObjectValue` both define `kind` and `url` on `Catalog`, `Namespace` and
    `Table`, so with both in scope write `ObjectValue::kind(&table)`.

## Pitfalls

| Wrong | Right |
| --- | --- |
| `path.split('.')` or `'.'.join(parts)` to build a path | pass the parts (`["lake", "eu west", "fills"]`) or quote (`lake."eu west".fills`) |
| `tables.get('eu west')` in JavaScript | `tables.get(['eu west'])` or `tables.get('"eu west"')` - bare text is dotted grammar |
| `catalog.create_table(...)` on a folder or memory catalog | register a `MediaTable` at the path and write through it |
| `tables.append("new", rows)` on a memory namespace expecting creation | append to an existing table; a memory level creates nothing |
| `warehouse.register(MediaTable("market.eu.x", ...))` under a `FolderCatalog` | the folder lists its own store: put the file in the folder, or register under a memory catalog |
| `len(catalog.tables)` / `tables.size` to test emptiness | `"x" in tables`, `tables.has('x')`, `is_empty()` - `len` drains the listing |
| expecting `table.kind()` to be `"table"` for a CSV leaf | it is the storage's kind, `"file"`; the object's `kind()` (Rust `ObjectValue::kind`) is `IOKind::Table` |
| reading `trades.csv` through `market.tables["trades.csv"]` | the extension comes off the name: `market.tables["trades"]` |
| `Catalog("x")` / `Table("x")` in Python | the kinds are never built directly: `MemoryCatalog`, `FolderCatalog`, `MediaTable` |
| `new warehouse.Catalog(...)` in JavaScript | `warehouse.Catalog.memory(...)`, `.folder(...)`, `.fromUrl(...)`, or `new warehouse.MemoryCatalog(...)` |
| `table.kind()` on a Rust `Table` with `IOBase` and `ObjectValue` both imported | `ObjectValue::kind(&table)` or `IOBase::kind(&table)` |
| cloning a `MediaTable::bound(path, Holder::buffer(..))` and reading the clone | keep the original; a buffer has no location to rebuild from |
| `Catalog::from_url` with `type = 'rest'` | this build has `memory` and `folder` catalogs, and `hadoop` under `iceberg`; the refusal names them |
| registering `lake` on `SystemWarehouse` in a test | a name unique to the process (`format!("test_{}", std::process::id())`), unregistered after |

## Language references

- `references/rust.md` - read when writing Rust (`Warehouse`, `FolderCatalog`, `MediaTable`, the traits in scope, `Holder::Table`).
- `references/python.md` - read when writing Python (`yggdryl.warehouse`, the mapping views, `**properties`, `IOBase` subclasses).
- `references/javascript.md` - read when writing Node.js (the `warehouse` namespace, static constructors, Map-like views, `IOBase.from`).

## Deeper

- The page: https://platob.github.io/yggdryl/warehouse/
- Objects as handles: https://platob.github.io/yggdryl/warehouse/#objects-as-handles
- Call counts: https://platob.github.io/yggdryl/warehouse/#performance
- Handles and backends: https://platob.github.io/yggdryl/holder/
- Locations and targets in a plan: https://platob.github.io/yggdryl/expression/plans/#locations-and-targets
- Sibling skills: `yggdryl-storage` (bytes, listings, backends), `yggdryl-records`
  (rows on a handle, `RecordOptions`), `yggdryl-expressions` (plans over a path).
