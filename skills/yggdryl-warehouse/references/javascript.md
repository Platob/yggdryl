# yggdryl-warehouse in JavaScript

`const { IOBase, warehouse } = require('yggdryl')`. The frozen `warehouse`
namespace holds one class per kind - `Catalog`, `Namespace`, `Table` - built
by a static constructor per implementation (`Catalog.memory`,
`Catalog.folder`, `Catalog.fromUrl`, `Namespace.memory`, `Namespace.folder`,
`Table.media`), with `implementation` naming it; `MemoryCatalog`,
`FolderCatalog`, `MemoryNamespace`, `FolderNamespace` and `MediaTable` are
constructors over those statics, callable with or without `new`; the Iceberg
implementations are `iceberg.IcebergCatalog`, `iceberg.IcebergNamespace` and
`iceberg.IcebergTable`. A path is dotted text or an array of parts; properties
are an ordered plain object, numbers and booleans spelled as text.

## Register a folder and resolve a path

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { IOBase, warehouse } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-skill-warehouse-'))
fs.mkdirSync(path.join(root, 'eu'))
fs.writeFileSync(path.join(root, 'eu', 'trades.csv'), 'symbol,price\nAAPL,187.5\nMSFT,410.25\n')

const registry = new warehouse.Warehouse()
registry.register(warehouse.Catalog.folder('market', root)) // a path, a Url or an IOBase

const trades = registry.table('market.eu.trades')
assert.ok(trades instanceof warehouse.Table)
assert.equal(trades.implementation, 'MediaTable')
assert.deepEqual(trades.path, ['market', 'eu', 'trades'])
assert.equal(trades.storage, 'text/csv')
assert.equal(trades.field().name, 'trades')
// The rows are read through the handle the table is.
const handle = IOBase.from(trades)
assert.equal(handle.rowSize(), 2)
assert.equal(handle.readArrowReader().intoTable().numRows, 2)
fs.rmSync(root, { recursive: true, force: true })
```

## Register a table at any location

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { IOBase, warehouse } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-skill-warehouse-'))
fs.writeFileSync(path.join(root, 'blob'), 'symbol,price\nAAPL,187.5\n')

// A declared schema is renamed after the table; a stated `media_type` types
// an extensionless location.
const table = warehouse.Table.media('lake.raw.trades', path.join(root, 'blob'), {
  dtype: 'struct<symbol: utf8, price: float64>',
  properties: { media_type: 'text/csv' },
})
const registry = new warehouse.Warehouse()
registry.register(table)
assert.equal(registry.catalog('lake').implementation, 'MemoryCatalog')
assert.equal(registry.namespace('lake.raw').implementation, 'MemoryNamespace')
const resolved = registry.table('lake.raw.trades')
assert.equal(resolved.field().name, 'trades')
assert.deepEqual(resolved.properties, { media_type: 'text/csv' })
assert.equal(IOBase.from(resolved).readArrowReader().intoTable().numRows, 1)

// A taken name conflicts; `replace` swaps and answers what was there.
assert.throws(() => registry.register(warehouse.Table.media('lake.raw.trades', path.join(root, 'other.csv'))), /got an existing table/)
assert.equal(String(registry.replace(warehouse.Table.media('lake.raw.trades', path.join(root, 'other.csv')))), 'lake.raw.trades')
assert.equal(String(registry.unregister('lake.raw.trades')), 'lake.raw.trades')
fs.rmSync(root, { recursive: true, force: true })
```

## Walk one level with the Map-like views

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { warehouse } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-skill-warehouse-'))
for (const leaf of ['trades.csv', 'eu/fills.csv', 'eu/lake/part-0.csv', 'README.md']) {
  fs.mkdirSync(path.dirname(path.join(root, leaf)), { recursive: true })
  fs.writeFileSync(path.join(root, leaf), 'symbol,price\nAAPL,187.5\n')
}
const market = warehouse.Catalog.folder('market', root)

assert.deepEqual([...market.namespaces().keys()], ['eu'])
assert.deepEqual([...market.tables()], ['trades']) // iterating a view yields its names
assert.equal(market.tables().has('eu.fills'), true)
assert.equal(market.namespaces().has('asia'), false)
const eu = market.namespaces().get('eu')
assert.equal(eu.implementation, 'FolderNamespace')
assert.deepEqual([...eu.tables().keys()], ['fills', 'lake'])
assert.equal(market.tables().get('eu.lake').storage, 'directory') // a folder under a namespace is a table
assert.deepEqual([...eu.tables().entries()].map(([name, table]) => [name, table.kind]), [['fills', 'table'], ['lake', 'table']])
assert.equal(eu.tables().size, 2)
assert.throws(() => market.tables().get('nowhere'), /expected a table at "market.nowhere", got nothing/)
// `children()` yields every child, whichever kind.
assert.deepEqual([...market.children()].map((child) => child.kind).sort(), ['namespace', 'table'])
fs.rmSync(root, { recursive: true, force: true })
```

## Write through a view

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const arrow = require('apache-arrow')
const { IOBase, warehouse } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-skill-warehouse-'))
const out = warehouse.Namespace.memory('lake.out', {
  objects: [warehouse.Table.media('lake.out.rows', path.join(root, 'rows.arrows'))],
})
const rows = new arrow.Table({ id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()) })

// Anything `BatchReader.from` reads, under the table's own options; a
// property bag or a `RecordOptions` beside the rows is this write's settings.
const written = out.tables().append('rows', rows)
assert.equal(String(written), 'lake.out.rows')
assert.equal(IOBase.from(written).rowSize(), 2)
assert.equal(IOBase.from(out.tables().append('rows', rows)).rowSize(), 4)
assert.equal(IOBase.from(out.tables().overwrite('rows', rows)).rowSize(), 2)

// A memory level creates nothing: register the table first.
assert.throws(() => out.tables().append('absent', rows), /"MemoryNamespace" does not support creating a table/)
fs.rmSync(root, { recursive: true, force: true })
```

## State properties once

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { warehouse } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-skill-warehouse-'))
fs.writeFileSync(path.join(root, 'trades.csv'), 'symbol,price\nAAPL,187.5\n')

const trades = warehouse.Table.media('lake.eu.trades', path.join(root, 'trades.csv'), { properties: { codec: 'gzip' } })
const eu = warehouse.Namespace.memory('lake.eu', { properties: { region: 'eu-west-1' }, objects: [trades] })
const lake = warehouse.Catalog.memory('lake', { properties: { token: 't', region: 'global' }, objects: [eu] })

// The parent's order, the child's values, the child's own appended.
assert.deepEqual(lake.table('eu.trades').properties, { token: 't', region: 'eu-west-1', codec: 'gzip' })
assert.deepEqual(Object.keys(lake.namespace('eu').properties), ['token', 'region'])

// The deepest registered object whose URL holds a location, on a path boundary.
const registry = new warehouse.Warehouse()
registry.register(lake)
assert.deepEqual(registry.propertiesFor(path.join(root, 'trades.csv')), { codec: 'gzip' })
assert.deepEqual(registry.propertiesFor(path.join(root, 'elsewhere.csv')), {})
fs.rmSync(root, { recursive: true, force: true })
```

## An object is a handle

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { IOBase, warehouse } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-skill-warehouse-'))
fs.mkdirSync(path.join(root, 'eu'))
fs.writeFileSync(path.join(root, 'eu', 'trades.csv'), 'symbol,price\nAAPL,187.5\n')

const held = IOBase.from(warehouse.Catalog.folder('market', root))
assert.equal(held.kind(), 'catalog')
assert.equal(held.isDir(), true)
assert.equal([...held.ls(true)].length, 2)
const trades = held.joinpath('eu/trades')
assert.equal(trades.kind(), 'file') // a leaf table's kind is its storage's
assert.equal(trades.rowSize(), 1)
assert.throws(() => held.readBytes(), /got a catalog/)
assert.throws(() => held.readArrowReader(), /name a table under it/)
fs.rmSync(root, { recursive: true, force: true })
```

## The system warehouse

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { IOBase, warehouse } = require('yggdryl')

assert.equal(warehouse.SystemWarehouse.catalog('local').implementation, 'MemoryCatalog')
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-skill-warehouse-'))
fs.writeFileSync(path.join(root, 'trades.csv'), 'symbol,price\nAAPL,187.5\n')
const catalog = `docs_skill_${process.pid}`
warehouse.SystemWarehouse.register(warehouse.Table.media([catalog, 'raw', 'trades'], path.join(root, 'trades.csv')))
try {
  assert.equal(IOBase.from(warehouse.SystemWarehouse.table([catalog, 'raw', 'trades'])).rowSize(), 1)
} finally {
  assert.equal(String(warehouse.SystemWarehouse.unregister([catalog])), catalog)
  fs.rmSync(root, { recursive: true, force: true })
}
```

## An Iceberg warehouse folder creates

`iceberg.IcebergCatalog` is the catalog that creates: namespaces nest to any
depth, a create descends through existing ones only, and each level keeps its
properties in its own document. It registers as the generic catalog
`intoCatalog()` answers. See https://platob.github.io/yggdryl/media/iceberg/#catalog.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const arrow = require('apache-arrow')
const { Field, IOBase, fields, iceberg, warehouse } = require('yggdryl')

const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-skill-warehouse-')), 'lake')
const schema = fields.struct('row', [Field.from('id: int64'), Field.from('venue: utf8')], { nullable: false })

// `create` writes `metadata/catalog.json`; `openOrCreate` absorbs the
// conflict a second `create` is.
const lake = iceberg.IcebergCatalog.create('lake', root)
assert.ok(fs.existsSync(path.join(root, 'metadata', 'catalog.json')))
assert.equal(iceberg.IcebergCatalog.openOrCreate('lake', root).name, 'lake')

// A create descends through existing namespaces only: `nyc` before `nyc.taxis`.
assert.throws(() => lake.tables().create('nyc.taxis', schema), /lake\.nyc/)
lake.namespaces().create('nyc', { owner: 'ops' })
assert.equal(lake.tables().create('nyc.taxis', schema).implementation, 'IcebergTable')
// The view's writes create a table from the rows' own schema.
lake.tables().append('nyc.zones', new arrow.Table({ id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()) }))

// What a namespace keeps is its own document: another catalog over the folder
// reads it, and `updateProperties` writes it.
const nyc = new iceberg.IcebergCatalog('lake', root).namespaces().get('nyc')
assert.deepEqual(nyc.properties, { owner: 'ops' })
nyc.updateProperties({ tier: 'gold' })
assert.equal(nyc.properties.tier, 'gold')
assert.deepEqual([...nyc.tables().keys()], ['taxis', 'zones'])

const registry = new warehouse.Warehouse()
registry.register(lake.intoCatalog())
assert.equal(IOBase.from(registry.table('lake.nyc.zones')).rowSize(), 2)
fs.rmSync(path.dirname(root), { recursive: true, force: true })
```

## Gotchas in JavaScript

- `name`, `path`, `kind`, `implementation`, `description`, `url`, `modified`
  (a `bigint`), `properties`, `storage`, `layout`, `declaredField` and
  `namespaceLevels` are getters; `namespaces()`, `tables()`, `children()`,
  `field()`, `get`, `resolve`, `table`, `namespace` are methods;
  `Warehouse.catalogs` is a getter and `SystemWarehouse.catalogs()` a
  static method.
- `tables.size` drains the listing; `[Symbol.iterator]` yields the names
  (`keys()`), `values()` and `entries()` open each name as one part.
- A `string` name is dotted grammar: `tables.get('eu west')` is refused as
  two parts; write `tables.get(['eu west'])` or `tables.get('"eu west"')`.
- An object has no record methods of its own: `IOBase.from(object)` (or
  `new IOBase(object)`) is the handle, and `IOBase.kind()` of a leaf table
  is `'file'` where the object's `kind` is `'table'`.
- An option an implementation has no use for is refused by name:
  `Catalog.memory('lake', { levels: 2 })` throws.
- `updateProperties` is refused by the memory, folder, media and S3 Tables
  implementations; an Iceberg catalog or namespace writes it into its own
  document.
- `iceberg.IcebergCatalog` is not a `warehouse.Catalog`: register
  `catalog.intoCatalog()`, and `iceberg.IcebergCatalog.from(catalog)` goes
  back.
- There is no XML for Analysis provider in the npm package: run the wheel's
  `yggdryl xmla serve name=location ...` (see the door table).
