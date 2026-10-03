import { Table as ArrowTable } from 'apache-arrow'

import {
  BatchReader,
  Field,
  IOBase,
  RecordOptions,
  Url,
  iceberg,
  warehouse,
  type Catalog,
  type HandleInput,
  type IcebergCatalog,
  type IcebergTable,
  type Namespace,
  type Namespaces,
  type ObjectOptions,
  type ObjectPathInput,
  type ObjectProperties,
  type SystemWarehouse,
  type Table,
  type Tables,
  type Warehouse,
  type WarehouseObject,
} from '..'

declare const schema: Field
declare const rows: ArrowTable
declare const handle: IOBase

// The static constructors, one per implementation, and the implementation
// constructors over them.
const options: ObjectOptions = {
  description: 'the lake',
  properties: { region: 'eu', retries: 3, verbose: true },
}
const memory: Catalog = warehouse.Catalog.memory('lake', options)
const folder: Catalog = warehouse.Catalog.folder('market', 'file:///lake', { levels: 2 })
const bound: Catalog = warehouse.Catalog.folder('market', handle)
const fromUrl: Catalog = warehouse.Catalog.fromUrl(Url.fromString('file:///lake'), {
  name: 'lake',
  type: 'folder',
})
const fromText: Catalog = warehouse.Catalog.fromUrl('/lake')
const raw: Namespace = warehouse.Namespace.memory(['lake', 'raw'], { objects: [memory] })
const eu: Namespace = warehouse.Namespace.folder('market.eu', '/lake/eu', { levels: 0 })
const trades: Table = warehouse.Table.media('lake.raw.trades', '/lake/eu/trades.csv', {
  field: schema,
  layout: 'leaf',
})
const typed: Table = warehouse.Table.media(['lake', 'raw', 'typed'], handle, {
  dtype: 'struct<id: int64>',
})
const built: Catalog = new warehouse.MemoryCatalog('lake', options)
const called: Catalog = warehouse.FolderCatalog('market', '/lake')
const namespaceBuilt: Namespace = new warehouse.MemoryNamespace('lake.raw')
const namespaceCalled: Namespace = warehouse.FolderNamespace(['market', 'eu'], handle, { levels: 1 })
const tableBuilt: Table = new warehouse.MediaTable('lake.raw.t', '/lake/t.parquet')
const implementationName: string = warehouse.MediaTable.name

// What every object answers.
const name: string = memory.name
const path: string[] = memory.path
const kind: string = memory.kind
const implementation: string = memory.implementation
const description: string | null = memory.description
const url: Url | null = folder.url
const modified: bigint | null = folder.modified
const properties: Record<string, string> = memory.properties
const bag: ObjectProperties = { region: 'eu' }
memory.updateProperties(bag, ['stale'])
memory.updateProperties({ region: 'us' })
memory.updateProperties()
const same: boolean = memory.equals(built)
const dotted: string = trades.toString()

// A catalog, and a namespace: the same minus the levels and the URL door.
const levels: number | null = folder.namespaceLevels
const namespaces: Namespaces = folder.namespaces()
const tables: Tables = folder.tables()
const children: Iterable<WarehouseObject> = folder.children()
for (const child of folder.children()) {
  const childKind: string = child.kind
  void childKind
}
const got: WarehouseObject = folder.get('eu')
const resolved: WarehouseObject = folder.resolve(['eu', 'trades'])
const pathInput: ObjectPathInput = 'eu.trades'
const table: Table = folder.table(pathInput)
const namespace: Namespace = folder.namespace('eu')
const createdNamespace: Namespace = folder.createNamespace('apac', { region: 'apac' })
const createdTable: Table = folder.createTable('fills', schema, { tier: 'raw' })
const createdFromText: Table = eu.createTable('fills', 'row: struct<id: int64>')
const namespaceTables: Tables = eu.tables()
const nested: Namespaces = eu.namespaces()
const euTable: Table = eu.table(['trades'])
const euNamespace: Namespace = eu.namespace('west')
const euGot: WarehouseObject = eu.get('trades')
const euResolved: WarehouseObject = eu.resolve('west.fills')

// A table: its schema and storage, its rows through the handle it is.
const field: Field = trades.field()
const storage: string = trades.storage
const layout: string = trades.layout
const declared: Field | null = trades.declaredField
const handleInput: HandleInput = trades
const tableHandle: IOBase = IOBase.from(trades)
const catalogHandle: IOBase = new IOBase(folder)
const namespaceHandle: IOBase = IOBase.from(eu)
const read: ArrowTable = tableHandle.readArrowReader().intoTable()

// The views are map-like and lazy.
const one: Namespace = namespaces.get('eu')
const byParts: Namespace = namespaces.get(['eu west'])
const has: boolean = namespaces.has('eu')
const size: number = namespaces.size
const keys: Iterable<string> = namespaces.keys()
for (const key of namespaces) {
  const text: string = key
  void text
}
for (const value of namespaces.values()) {
  const valueName: string = value.name
  void valueName
}
for (const [key, value] of namespaces.entries()) {
  const pair: readonly [string, Namespace] = [key, value]
  void pair
}
const createdView: Namespace = namespaces.create('apac', { region: 'apac' })
const openedView: Namespace = namespaces.openOrCreate('apac')
const oneTable: Table = tables.get('eu.trades')
const tableByParts: Table = tables.get(['eu', 'trades'])
const hasTable: boolean = tables.has(['eu', 'trades'])
const tableCount: number = tables.size
const tableKeys: Iterable<string> = tables.keys()
for (const [key, value] of tables.entries()) {
  const pair: readonly [string, Table] = [key, value]
  void pair
}
const createdTableView: Table = tables.create('fills', schema)
const openedTable: Table = tables.openOrCreate('fills', 'row: struct<id: int64>', { tier: 'raw' })
const appended: Table = tables.append('eu.trades', rows)
const appendedReader: Table = tables.append('eu.trades', BatchReader.from(rows), new RecordOptions('text/csv'))
const appendedBag: Table = tables.append('eu.trades', rows, { safe: true })
const overwritten: Table = tables.overwrite('eu.trades', rows, new RecordOptions('text/csv'), { safe: true })

// The registry, and the process's one.
const registry: Warehouse = new warehouse.Warehouse()
registry.register(memory)
registry.register(trades)
const replaced: WarehouseObject | null = registry.replace(folder)
const removed: WarehouseObject = registry.unregister('lake.raw.trades')
const catalogs: Catalog[] = registry.catalogs
const catalog: Catalog = registry.catalog('lake')
const object: WarehouseObject = registry.get(['lake', 'raw'])
const registered: Table = registry.table('lake.raw.trades')
const registeredNamespace: Namespace = registry.namespace('lake.raw')
const propertiesFor: Record<string, string> = registry.propertiesFor('/lake/eu/trades.csv')
const propertiesForUrl: Record<string, string> = registry.propertiesFor(Url.fromString('file:///lake'))

const system: typeof SystemWarehouse = warehouse.SystemWarehouse
system.register(trades)
const systemReplaced: WarehouseObject | null = system.replace(trades)
const systemRemoved: WarehouseObject = system.unregister(['lake', 'raw', 'trades'])
const systemCatalogs: Catalog[] = system.catalogs()
const local: Catalog = system.catalog('local')
const temporary: WarehouseObject = system.get('local.temporary')
const systemTable: Table = system.table('lake.raw.trades')
const systemNamespace: Namespace = system.namespace(['local', 'temporary'])
const systemProperties: Record<string, string> = system.propertiesFor('/tmp/x')

// The Iceberg classes keep their names under the iceberg namespace.
const icebergCatalog: IcebergCatalog = new iceberg.Catalog('/lake')
const icebergTable: IcebergTable = iceberg.Table.open('/lake/trades')

void bound
void fromUrl
void fromText
void raw
void typed
void called
void namespaceBuilt
void namespaceCalled
void tableBuilt
void implementationName
void name
void path
void kind
void implementation
void description
void url
void modified
void properties
void same
void dotted
void levels
void children
void got
void resolved
void table
void namespace
void createdNamespace
void createdTable
void createdFromText
void namespaceTables
void nested
void euTable
void euNamespace
void euGot
void euResolved
void field
void storage
void layout
void declared
void handleInput
void catalogHandle
void namespaceHandle
void read
void one
void byParts
void has
void size
void keys
void createdView
void openedView
void oneTable
void tableByParts
void hasTable
void tableCount
void tableKeys
void createdTableView
void openedTable
void appended
void appendedReader
void appendedBag
void overwritten
void replaced
void removed
void catalogs
void catalog
void object
void registered
void registeredNamespace
void propertiesFor
void propertiesForUrl
void systemReplaced
void systemRemoved
void systemCatalogs
void local
void temporary
void systemTable
void systemNamespace
void systemProperties
void icebergCatalog
void icebergTable
