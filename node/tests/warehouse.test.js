'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')

const { Field, IOBase, Plan, Url, iceberg, warehouse } = require('yggdryl')

const { Catalog, Namespace, Table, Namespaces, Tables, Warehouse, SystemWarehouse } = warehouse

// A folder is a catalog: its folders are namespaces, its files tables. One
// folder carries a space, so the path grammar's quoting is exercised, and one
// leaf is no table at all.
function lake(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-warehouse-'))
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  fs.mkdirSync(path.join(root, 'eu'))
  fs.mkdirSync(path.join(root, 'eu west'))
  fs.mkdirSync(path.join(root, 'us'))
  fs.writeFileSync(path.join(root, 'eu', 'trades.csv'), 'symbol,price\nAAPL,187.5\n')
  fs.writeFileSync(path.join(root, 'eu west', 'fills.csv'), 'symbol,qty\nAAPL,10\nMSFT,20\n')
  fs.writeFileSync(path.join(root, 'us', 'quotes.csv'), 'symbol,bid\nMSFT,410\n')
  fs.writeFileSync(path.join(root, 'README.md'), '# not a table\n')
  fs.writeFileSync(path.join(root, '.hidden.csv'), 'a\n1\n')
  return root
}

test('the warehouse namespace holds one class per kind and a constructor per implementation', () => {
  assert.deepEqual(Object.keys(warehouse).sort(), [
    'Catalog',
    'FolderCatalog',
    'FolderNamespace',
    'MediaTable',
    'MemoryCatalog',
    'MemoryNamespace',
    'Namespace',
    'Namespaces',
    'SystemWarehouse',
    'Table',
    'Tables',
    'Warehouse',
  ])
  assert.ok(Object.isFrozen(warehouse))
  for (const name of ['Catalog', 'Namespace', 'Table', 'Namespaces', 'Tables', 'Warehouse']) {
    assert.equal(typeof warehouse[name], 'function')
    assert.equal(warehouse[name].name, name)
  }
  // The implementation constructors answer the kind's class.
  const built = new warehouse.MemoryCatalog('lake', { description: 'the lake' })
  assert.ok(built instanceof Catalog)
  assert.equal(built.implementation, 'MemoryCatalog')
  assert.equal(built.description, 'the lake')
  assert.equal(warehouse.MemoryCatalog.name, 'MemoryCatalog')
  assert.ok(warehouse.FolderCatalog('market', 'file:///lake') instanceof Catalog)
  // The raw native exports are gone: the namespace is the one spelling.
  const yggdryl = require('yggdryl')
  for (const name of ['Catalog', 'Namespace', 'Table', 'Tables', 'Namespaces', 'Warehouse', 'SystemWarehouse']) {
    assert.equal(yggdryl[name], undefined, name)
  }
})

test('the iceberg namespace keeps its names under the renamed native classes', () => {
  for (const name of ['Catalog', 'Namespace', 'Namespaces', 'Tables', 'Table']) {
    assert.equal(typeof iceberg[name], 'function', name)
    assert.notEqual(iceberg[name], warehouse[name], name)
  }
  assert.equal(iceberg.Catalog.name, 'IcebergCatalog')
  assert.equal(iceberg.Table.name, 'IcebergTable')
  assert.equal(typeof iceberg.Table.create, 'function')
  assert.equal(typeof iceberg.Namespaces.prototype.values, 'function')
  assert.equal(typeof iceberg.Tables.prototype.entries, 'function')
})

test('a memory catalog describes itself and touches nothing', () => {
  const catalog = Catalog.memory('lake', {
    description: 'registered objects',
    properties: { region: 'eu', retries: 3, verbose: true },
  })
  assert.equal(catalog.name, 'lake')
  assert.deepEqual(catalog.path, ['lake'])
  assert.equal(catalog.kind, 'catalog')
  assert.equal(catalog.implementation, 'MemoryCatalog')
  assert.equal(catalog.description, 'registered objects')
  assert.equal(catalog.url, null)
  assert.equal(catalog.modified, null)
  assert.equal(catalog.namespaceLevels, null)
  // Properties are an ordered plain object, numbers and booleans spelled as text.
  assert.deepEqual(catalog.properties, { region: 'eu', retries: '3', verbose: 'true' })
  assert.deepEqual(Object.keys(catalog.properties), ['region', 'retries', 'verbose'])
  assert.equal(String(catalog), 'lake')
  assert.equal(catalog.namespaces().size, 0)
  assert.equal(catalog.tables().size, 0)
  assert.deepEqual([...catalog.children()], [])
  assert.ok(catalog.equals(Catalog.memory('lake', { description: 'registered objects', properties: { region: 'eu', retries: '3', verbose: 'true' } })))
  assert.ok(!catalog.equals(Catalog.memory('other')))
  // An option the implementation has no use for is refused by name.
  assert.throws(() => Catalog.memory('lake', { levels: 2 }), /expected no `levels` option on a memory catalog/)
  assert.throws(() => Namespace.memory('lake.raw', { layout: 'leaf' }), /expected no `layout` option on a memory namespace/)
  // What it keeps for itself is nothing, so updating refuses by name.
  assert.throws(() => catalog.updateProperties({ region: 'us' }), /MemoryCatalog/)
  // Creation refuses by name, too: a memory catalog lists what was registered.
  assert.throws(() => catalog.createNamespace('raw'), /"MemoryCatalog" does not support creating a namespace/)
  assert.throws(() => catalog.createTable('t', Field.from('row: struct<id: int64>')), /"MemoryCatalog" does not support creating a table/)
})

test('a memory namespace and a media table register under a memory catalog', (t) => {
  const root = lake(t)
  const trades = Table.media('lake.raw.trades', path.join(root, 'eu', 'trades.csv'), {
    description: 'one day of trades',
    properties: { source: 'csv' },
  })
  assert.equal(trades.name, 'trades')
  assert.deepEqual(trades.path, ['lake', 'raw', 'trades'])
  assert.equal(trades.kind, 'table')
  assert.equal(trades.implementation, 'MediaTable')
  assert.equal(trades.layout, 'leaf')
  assert.equal(trades.storage, 'text/csv')
  assert.equal(trades.declaredField, null)
  assert.equal(trades.url.toString(), Url.fromPath(path.join(root, 'eu', 'trades.csv')).toString())
  assert.equal(String(trades), 'lake.raw.trades')
  assert.equal(trades.field().fieldLen, 2)
  assert.deepEqual(trades.properties, { source: 'csv' })

  const raw = Namespace.memory(['lake', 'raw'], { objects: [trades], properties: { tier: 'raw' } })
  assert.equal(raw.implementation, 'MemoryNamespace')
  assert.deepEqual(raw.path, ['lake', 'raw'])
  assert.equal(raw.kind, 'namespace')
  assert.equal(String(raw), 'lake.raw')
  const catalog = Catalog.memory('lake', { objects: [raw], properties: { region: 'eu' } })
  assert.deepEqual([...catalog.namespaces().keys()], ['raw'])
  assert.deepEqual([...catalog.tables()], [])
  // The child answered carries its parent's properties, then its own.
  const listed = catalog.table('raw.trades')
  assert.deepEqual(listed.properties, { region: 'eu', tier: 'raw', source: 'csv' })
  assert.deepEqual(catalog.namespace('raw').properties, { region: 'eu', tier: 'raw' })
  assert.deepEqual(catalog.namespaces().get('raw').tables().get('trades').properties, {
    region: 'eu',
    tier: 'raw',
    source: 'csv',
  })
  // A namespace must sit under a catalog, a table must have a name.
  assert.throws(() => Namespace.memory('alone'), /at least two parts/)
  assert.throws(() => Table.media('', path.join(root, 'eu', 'trades.csv')), /empty path/)
  assert.throws(() => Namespace.memory('lake.raw', { objects: [catalog] }), /one level below/)
  // A declared field is renamed after the table.
  const declared = Table.media('lake.raw.declared', path.join(root, 'eu', 'trades.csv'), {
    field: 'row: struct<symbol: utf8, price: float64>',
  })
  assert.equal(declared.declaredField.name, 'declared')
  assert.equal(declared.field().name, 'declared')
  const typed = Table.media('lake.raw.typed', path.join(root, 'eu', 'trades.csv'), {
    dtype: 'struct<symbol: utf8, price: float64>',
  })
  assert.equal(typed.field().nullable, false)
  assert.throws(() => Table.media('lake.raw.t', root, { layout: 'bogus' }), /expected a layout of `leaf`, `folder` or `format`/)
  assert.equal(Table.media('lake.raw.t', root, { layout: 'folder' }).storage, 'directory')
})

test('a folder catalog lists namespaces and tables and reads rows through a handle', (t) => {
  const root = lake(t)
  const market = Catalog.folder('market', root, { properties: { region: 'eu' } })
  assert.equal(market.implementation, 'FolderCatalog')
  assert.equal(market.kind, 'catalog')
  assert.equal(market.namespaceLevels, 1)
  assert.equal(market.url.toString(), Url.fromPath(root).toString())
  assert.equal(typeof market.modified, 'bigint')
  assert.deepEqual(market.properties, { region: 'eu' })

  // Namespaces and tables are two views over one level; the README is no table.
  assert.deepEqual([...market.namespaces().keys()].sort(), ['eu', 'eu west', 'us'])
  assert.deepEqual([...market.tables()], [])
  assert.deepEqual(
    [...market.children()]
      .map((child) => [child.kind, String(child)])
      .sort(([, left], [, right]) => (left < right ? -1 : 1)),
    [
      ['namespace', 'market."eu west"'],
      ['namespace', 'market.eu'],
      ['namespace', 'market.us'],
    ],
  )
  const eu = market.namespaces().get('eu')
  assert.ok(eu instanceof Namespace)
  assert.equal(eu.implementation, 'FolderNamespace')
  assert.deepEqual(eu.path, ['market', 'eu'])
  assert.deepEqual([...eu.tables().keys()], ['trades'])
  assert.equal(eu.tables().size, 1)
  assert.equal(eu.namespaces().size, 0)
  // Properties propagate: the catalog's reach the namespace and the table.
  assert.deepEqual(eu.properties, { region: 'eu' })
  const trades = eu.tables().get('trades')
  assert.ok(trades instanceof Table)
  assert.equal(trades.implementation, 'MediaTable')
  assert.deepEqual(trades.path, ['market', 'eu', 'trades'])
  assert.equal(trades.storage, 'text/csv')
  assert.deepEqual(trades.properties, { region: 'eu' })
  assert.equal(trades.field().fieldLen, 2)

  // The path grammar quotes a part carrying a space; parts need no quoting.
  const fills = market.table('"eu west".fills')
  assert.equal(fills.name, 'fills')
  assert.equal(String(fills), 'market."eu west".fills')
  assert.ok(fills.equals(market.table(['eu west', 'fills'])))
  assert.ok(fills.equals(market.resolve(['eu west', 'fills'])))
  assert.equal(market.resolve('us').kind, 'namespace')
  assert.equal(market.get('us').kind, 'namespace')

  // The rows are read through the handle the table is.
  const handle = IOBase.from(trades)
  assert.equal(handle.kind(), 'file')
  const rows = handle.readArrowReader().intoTable()
  assert.equal(rows.numRows, 1)
  assert.deepEqual(rows.schema.fields.map((field) => field.name), ['symbol', 'price'])
  assert.equal(new IOBase(fills).readArrowReader().intoTable().numRows, 2)
  assert.equal(IOBase.from(fills).rowSize(), 2)

  // Absence names the path; a namespace where a table is asked for is absence.
  assert.throws(() => market.table('eu.missing'), /expected a table at "market.eu.missing", got nothing/)
  assert.throws(() => market.table('eu'), /expected a table at "market.eu", got nothing/)
  assert.throws(() => market.namespace('eu.trades'), /expected a namespace at "market.eu.trades", got nothing/)
  assert.equal(market.tables().has('eu.trades'), true)
  assert.equal(market.tables().has('eu'), false)
  assert.equal(market.namespaces().has('eu'), true)
  assert.equal(market.namespaces().has('eu.trades'), false)
  // A folder catalog creates nothing.
  assert.throws(() => market.namespaces().create('apac'), /"FolderCatalog" does not support creating a namespace/)
  assert.throws(() => eu.tables().create('fills', 'row: struct<id: int64>'), /"FolderNamespace" does not support creating a table/)
  assert.equal(market.namespaces().openOrCreate('eu').name, 'eu')
  assert.equal(eu.tables().openOrCreate('trades', 'row: struct<id: int64>').name, 'trades')
})

test('a folder catalog over a handle binds to it, and the levels decide what a folder is', (t) => {
  const root = lake(t)
  const bound = Catalog.folder('market', new IOBase(root))
  assert.equal(bound.implementation, 'FolderCatalog')
  assert.deepEqual([...bound.namespaces().keys()].sort(), ['eu', 'eu west', 'us'])
  // With no namespace level, every folder is a table read as the rows beneath it.
  const flat = Catalog.folder('market', root, { levels: 0 })
  assert.equal(flat.namespaceLevels, 0)
  assert.deepEqual([...flat.namespaces()], [])
  assert.deepEqual([...flat.tables().keys()].sort(), ['eu', 'eu west', 'us'])
  const eu = flat.tables().get('eu')
  assert.equal(eu.storage, 'directory')
  assert.equal(eu.layout, 'folder')
  assert.equal(IOBase.from(eu).readArrowReader().intoTable().numRows, 1)
  // A folder namespace stands on its own, over a container, with no level under it.
  const us = Namespace.folder('market.us', path.join(root, 'us'))
  assert.equal(us.implementation, 'FolderNamespace')
  assert.deepEqual([...us.tables().keys()], ['quotes'])
  // A leaf is a table at any level; a level only decides what a folder is.
  const leveled = new warehouse.FolderNamespace(['market', 'us'], new IOBase(path.join(root, 'us')), { levels: 1 })
  assert.equal(leveled.tables().size, 1)
  assert.equal(leveled.namespaces().size, 0)
  assert.throws(() => Namespace.folder('alone', root), /at least two parts/)
})

test('a catalog is read from a URL under a property bag', (t) => {
  const root = lake(t)
  const folder = Catalog.fromUrl(Url.fromPath(root), { region: 'eu' })
  assert.equal(folder.implementation, 'FolderCatalog')
  assert.equal(folder.name, path.basename(root))
  assert.deepEqual(folder.properties, { region: 'eu' })
  const named = Catalog.fromUrl(root, { name: 'market' })
  assert.equal(named.name, 'market')
  const memory = Catalog.fromUrl(root, { type: 'memory', name: 'lake' })
  assert.equal(memory.implementation, 'MemoryCatalog')
  assert.throws(() => Catalog.fromUrl(root, { type: 'rest' }), /this build has no catalog of that type/)
})

test('the collection views are map-like and lazy', (t) => {
  const root = lake(t)
  const market = Catalog.folder('market', root)
  const namespaces = market.namespaces()
  assert.ok(namespaces instanceof Namespaces)
  assert.equal(typeof namespaces[Symbol.iterator], 'function')
  assert.deepEqual([...namespaces].sort(), ['eu', 'eu west', 'us'])
  assert.deepEqual([...namespaces.keys()].sort(), ['eu', 'eu west', 'us'])
  assert.deepEqual([...namespaces.values()].map((namespace) => namespace.name).sort(), ['eu', 'eu west', 'us'])
  // A name is read as the one part it is, or as dotted text through the grammar.
  assert.equal(namespaces.get(['eu west']).name, 'eu west')
  assert.equal(namespaces.get('"eu west"').name, 'eu west')
  assert.equal(namespaces.has(['eu west']), true)
  assert.equal(namespaces.has(['eu', 'trades']), false)
  assert.equal(market.tables().has(['eu', 'trades']), true)
  assert.equal(market.tables().get(['eu west', 'fills']).name, 'fills')
  assert.throws(() => namespaces.get('eu west'), /expected a path of parts, got "eu west"/)
  assert.deepEqual(
    [...namespaces.entries()]
      .map(([name, namespace]) => [name, String(namespace)])
      .sort(([left], [right]) => (left < right ? -1 : 1)),
    [
      ['eu', 'market.eu'],
      ['eu west', 'market."eu west"'],
      ['us', 'market.us'],
    ],
  )
  assert.equal(namespaces.size, 3)
  const tables = namespaces.get('eu').tables()
  assert.ok(tables instanceof Tables)
  assert.deepEqual([...tables], ['trades'])
  assert.deepEqual([...tables.values()].map(String), ['market.eu.trades'])
  assert.deepEqual([...tables.entries()].map(([name, table]) => [name, table.storage]), [['trades', 'text/csv']])
  assert.equal(tables.size, 1)
  assert.throws(() => tables.get('missing'), /expected a table at "market.eu.missing", got nothing/)
  // The view is lazy: a table written after the view was built is found.
  fs.writeFileSync(path.join(root, 'eu', 'quotes.csv'), 'symbol,bid\nAAPL,187\n')
  assert.deepEqual([...tables.keys()].sort(), ['quotes', 'trades'])
  assert.equal(tables.size, 2)
  // Dotted names descend from the catalog's own view.
  assert.equal(market.tables().get('eu.quotes').name, 'quotes')
  assert.equal(market.tables().has('eu.quotes'), true)
  // The names iterator is spent once walked.
  const keys = tables.keys()
  assert.equal([...keys].length, 2)
  assert.deepEqual([...keys], [])
})

test('a table write through the view takes what every record write takes', (t) => {
  const root = lake(t)
  const market = Catalog.folder('market', root)
  const eu = market.namespaces().get('eu')
  // Rows as an Arrow table, appended to an existing table under its own options.
  const arrow = require('apache-arrow')
  const more = new arrow.Table({
    symbol: arrow.vectorFromArray(['MSFT'], new arrow.Utf8()),
    price: arrow.vectorFromArray([410.25], new arrow.Float64()),
  })
  const appended = eu.tables().append('trades', more)
  assert.ok(appended instanceof Table)
  assert.equal(String(appended), 'market.eu.trades')
  assert.equal(IOBase.from(appended).readArrowReader().intoTable().numRows, 2)
  // The dotted spelling from the catalog's view, and a property bag on a copy of the table's options.
  const replaced = market.tables().overwrite('eu.trades', more, { safe: true })
  assert.equal(IOBase.from(replaced).readArrowReader().intoTable().numRows, 1)
  // A table nothing creates is refused by name, before any row is read.
  assert.throws(() => eu.tables().append('missing', more), /"FolderNamespace" does not support creating a table/)
  // Rows that name no Arrow shape are refused by `BatchReader.from`.
  assert.throws(() => eu.tables().append('trades', 42), TypeError)
})

test('a warehouse registers objects and resolves paths', (t) => {
  const root = lake(t)
  const registry = new Warehouse()
  assert.deepEqual(registry.catalogs, [])
  const trades = Table.media('lake.raw.trades', path.join(root, 'eu', 'trades.csv'))
  registry.register(Catalog.memory('lake', { properties: { region: 'eu' } }))
  registry.register(trades)
  // The memory namespace along the path was created, and it inherits.
  assert.deepEqual(registry.catalogs.map(String), ['lake'])
  assert.equal(registry.catalog('lake').implementation, 'MemoryCatalog')
  assert.equal(registry.get('lake.raw').kind, 'namespace')
  assert.equal(registry.namespace(['lake', 'raw']).implementation, 'MemoryNamespace')
  assert.deepEqual(registry.namespace('lake.raw').properties, { region: 'eu' })
  // The table answered carries what it inherits, so it is the registered
  // one at its path and location, under the catalog's properties.
  const resolved = registry.table('lake.raw.trades')
  assert.ok(!resolved.equals(trades))
  assert.equal(String(resolved), String(trades))
  assert.equal(resolved.url.toString(), trades.url.toString())
  assert.deepEqual(resolved.properties, { region: 'eu' })
  assert.equal(IOBase.from(resolved).readArrowReader().intoTable().numRows, 1)
  // A table registers its own catalog and namespaces when none is there.
  registry.register(Table.media(['other', 'a', 'b', 'c'], path.join(root, 'us', 'quotes.csv')))
  assert.deepEqual(registry.catalogs.map(String), ['lake', 'other'])
  assert.equal(registry.table('other.a.b.c').storage, 'text/csv')
  // A folder catalog registers by name and lists its own store.
  registry.register(Catalog.folder('market', root))
  assert.equal(registry.table('market.eu.trades').field().fieldLen, 2)
  assert.throws(() => registry.register(Table.media('market.eu.more', root)), /lists its own store/)
  // Conflicts, replacements and removals.
  assert.throws(() => registry.register(Catalog.memory('lake')), /expected to create a catalog at "lake", got an existing catalog/)
  assert.throws(() => registry.register(trades), /expected to create a table at "lake.raw.trades", got an existing table/)
  assert.equal(registry.replace(Catalog.memory('fresh')), null)
  const replaced = registry.replace(Table.media('lake.raw.trades', path.join(root, 'us', 'quotes.csv')))
  assert.ok(replaced.equals(trades))
  assert.equal(registry.table('lake.raw.trades').field().fieldLen, 2)
  assert.equal(registry.unregister('lake.raw.trades').kind, 'table')
  assert.throws(() => registry.table('lake.raw.trades'), /expected a child at "lake.raw.trades", got nothing/)
  assert.throws(() => registry.unregister('lake.raw.trades'), /got nothing/)
  assert.equal(registry.unregister(['fresh']).kind, 'catalog')
  assert.throws(() => registry.catalog('fresh'), /expected a catalog at "fresh", got nothing/)
  // The properties of the deepest registered object holding a URL, on a path
  // boundary: what that object itself states.
  assert.deepEqual(registry.propertiesFor(Url.fromPath(path.join(root, 'eu', 'trades.csv'))), {})
  registry.register(Table.media('lake.raw.eu', path.join(root, 'eu'), { layout: 'folder', properties: { tier: 'raw' } }))
  assert.deepEqual(registry.propertiesFor(path.join(root, 'eu', 'trades.csv')), { tier: 'raw' })
  assert.deepEqual(registry.propertiesFor(path.join(root, 'eu')), { tier: 'raw' })
  assert.deepEqual(registry.propertiesFor(path.join(root, 'europe')), {})
})

test('the system warehouse starts with the local catalog and registers like any other', (t) => {
  const root = lake(t)
  const local = SystemWarehouse.catalog('local')
  assert.ok(local instanceof Catalog)
  assert.equal(local.implementation, 'MemoryCatalog')
  assert.ok(SystemWarehouse.catalogs().some((catalog) => catalog.name === 'local'))
  const temporary = SystemWarehouse.get('local.temporary')
  assert.equal(temporary.kind, 'namespace')
  assert.equal(temporary.implementation, 'FolderNamespace')
  assert.equal(SystemWarehouse.namespace(['local', 'temporary']).name, 'temporary')

  const name = `warehouse_test_${process.pid}`
  const trades = Table.media([name, 'raw', 'trades'], path.join(root, 'eu', 'trades.csv'))
  SystemWarehouse.register(trades)
  t.after(() => {
    try {
      SystemWarehouse.unregister([name])
    } catch {
      // Already removed below.
    }
  })
  assert.ok(SystemWarehouse.table(`${name}.raw.trades`).equals(trades))
  assert.equal(SystemWarehouse.get([name, 'raw']).kind, 'namespace')
  assert.equal(IOBase.from(SystemWarehouse.table([name, 'raw', 'trades'])).readArrowReader().intoTable().numRows, 1)
  assert.deepEqual(SystemWarehouse.propertiesFor(path.join(root, 'eu', 'trades.csv')), {})
  assert.throws(() => SystemWarehouse.register(trades), /got an existing table/)
  assert.equal(SystemWarehouse.replace(Table.media([name, 'raw', 'trades'], path.join(root, 'us', 'quotes.csv'))).name, 'trades')
  assert.equal(SystemWarehouse.unregister([name, 'raw', 'trades']).kind, 'table')
  assert.equal(SystemWarehouse.unregister([name]).kind, 'catalog')
  assert.throws(() => SystemWarehouse.table(`${name}.raw.trades`), /got nothing/)
})

test('an object is held as the handle it is', (t) => {
  const root = lake(t)
  const market = Catalog.folder('market', root)
  const handle = IOBase.from(market)
  assert.equal(handle.kind(), 'catalog')
  assert.equal(handle.url.toString(), Url.fromPath(root).toString())
  assert.deepEqual([...handle.ls()].map((child) => child.kind()).sort(), ['namespace', 'namespace', 'namespace'])
  const eu = IOBase.from(market.namespaces().get('eu'))
  assert.equal(eu.kind(), 'namespace')
  assert.equal(new IOBase(market.namespaces().get('eu')).kind(), 'namespace')
  const children = [...eu.ls()]
  assert.equal(children.length, 1)
  assert.equal(children[0].kind(), 'file')
  assert.equal(children[0].readArrowReader().intoTable().numRows, 1)
  // A container object holds no bytes and is no table.
  assert.throws(() => eu.readBytes(), /pread|read_all_bytes|atomic/)
  assert.throws(() => eu.readArrowReader(), /expected a table, got the namespace `market.eu`/)
  // A child by path descends the hierarchy.
  assert.equal(handle.joinpath('eu', 'trades').readArrowReader().intoTable().numRows, 1)
  // A memory object has no location and lists what was registered.
  const lake_ = IOBase.from(Catalog.memory('lake', { objects: [Namespace.memory('lake.raw')] }))
  assert.equal(lake_.kind(), 'catalog')
  assert.equal(lake_.url, null)
  assert.deepEqual([...lake_.ls()].map((child) => child.kind()), ['namespace'])
})

test('a plan reads the table registered at its path, in the system warehouse or a given one', (t) => {
  const root = lake(t)
  const name = `warehouse_plan_${process.pid}`
  const trades = Table.media([name, 'eu', 'trades'], path.join(root, 'eu', 'trades.csv'))
  SystemWarehouse.register(trades)
  t.after(() => {
    try {
      SystemWarehouse.unregister([name])
    } catch {
      // Already removed below.
    }
  })
  const read = new Plan(`select symbol from ${name}.eu.trades where price > 100`)
  assert.deepEqual([...read.execute().intoTable().getChild('symbol')], ['AAPL'])
  // A write to a registered table lands where the table is.
  SystemWarehouse.register(Table.media([name, 'eu', 'copy'], path.join(root, 'eu', 'copy.csv')))
  new Plan(`insert into ${name}.eu.copy select * from ${name}.eu.trades`).execute().intoTable()
  assert.equal(IOBase.from(SystemWarehouse.table(`${name}.eu.copy`)).rowSize(), 1)
  SystemWarehouse.unregister([name])
  // Unregistered, the path is nothing, and the refusal says what to do.
  assert.throws(() => read.execute(), /register the table or name a URL/)

  // A given warehouse is read instead of the process's own.
  const registry = new Warehouse()
  registry.register(Table.media('lake.us.quotes', path.join(root, 'us', 'quotes.csv')))
  const quotes = new Plan('select bid from lake.us.quotes')
  assert.deepEqual([...quotes.executeIn(registry).intoTable().getChild('bid')], [410n])
  assert.throws(() => quotes.execute(), /expected a table at "lake.us.quotes", got nothing/)
  assert.throws(() => new Plan('select * from lake.us').executeIn(registry), /got the namespace `lake.us`/)

  // A URL may stand unquoted after `from`, printed back quoted.
  const location = path.join(root, 'us', 'quotes.csv')
  const unquoted = new Plan(`select symbol from ${location}`)
  assert.equal(unquoted.toString(), `select symbol from '${Url.fromPath(location)}'`)
  assert.deepEqual([...unquoted.execute().intoTable().getChild('symbol')], ['MSFT'])
})
