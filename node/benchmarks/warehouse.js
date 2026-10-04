'use strict'

// What reaching a table through the warehouse costs, beside the calls it
// wraps.
//
// Every row here is a *boundary* measurement over the same corpus: a folder
// of namespaces of CSV tables. The registry rows resolve a path against
// objects registered in memory, so the pair isolates the path intake - the
// grammar for dotted text, the parts as they are - and the enum crossing; the
// folder rows list a container as a catalog does, and the `node:fs` row
// beside them is the trusted baseline because the folder catalog's listing is
// one `readdir` per level, so what the pair isolates is the catalog boundary
// rather than the file system underneath. The record row compares a read
// through the handle a table is against the same read through a local
// handle, which is the same core code over the same mapped file.

const fs = require('node:fs')
const os = require('node:os')
const { performance } = require('node:perf_hooks')
const path = require('node:path')
const { IOBase, warehouse } = require('yggdryl')

const { Catalog, Table, Warehouse } = warehouse

// A path resolution is far less work than a byte round trip, so this target
// counts in tens of thousands by default.
const iterations = Number.parseInt(
  process.env.YGGDRYL_BENCH_ITERATIONS ?? '20000',
  10,
)
if (!Number.isSafeInteger(iterations) || iterations <= 0) {
  throw new RangeError(
    'YGGDRYL_BENCH_ITERATIONS must be a positive safe integer',
  )
}

function benchmark(name, operation) {
  for (let index = 0; index < Math.min(iterations, 100); index += 1) operation()
  const started = performance.now()
  for (let index = 0; index < iterations; index += 1) operation()
  const elapsed = performance.now() - started
  const rate = Math.round((iterations * 1_000) / elapsed)
  console.log(`${name}: ${rate.toLocaleString('en-US')} operations/second`)
}

// Every fixture is built once, outside the measured loops, so the numbers
// report the boundary crossing rather than the corpus: eight namespaces of
// eight tables, one row each.
const NAMESPACES = 8
const TABLES = 8

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-warehouse-bench-'))
for (let region = 0; region < NAMESPACES; region += 1) {
  const folder = path.join(root, `region${region}`)
  fs.mkdirSync(folder)
  for (let table = 0; table < TABLES; table += 1) {
    fs.writeFileSync(path.join(folder, `table${table}.csv`), 'symbol,price\nAAPL,187.5\n')
  }
}
const leaf = path.join(root, 'region0', 'table0.csv')

// The registry: one memory catalog, every table registered at its path.
const registry = new Warehouse()
registry.register(Catalog.memory('lake'))
for (let region = 0; region < NAMESPACES; region += 1) {
  for (let table = 0; table < TABLES; table += 1) {
    registry.register(
      Table.media(['lake', `region${region}`, `table${table}`], path.join(root, `region${region}`, `table${table}.csv`)),
    )
  }
}

// The folder catalog: the same corpus, listed as it is asked.
const market = Catalog.folder('market', root)

// Construction: describing an object touches nothing.
benchmark('warehouse/memory_catalog', () => Catalog.memory('lake'))
benchmark('warehouse/folder_catalog', () => Catalog.folder('market', root))
benchmark('warehouse/media_table', () => Table.media('lake.region0.table0', leaf))

// Resolution against the registry: dotted text through the grammar, and the
// parts as they are.
benchmark('warehouse/resolve_registered_text', () => registry.table('lake.region7.table7'))
benchmark('warehouse/resolve_registered_parts', () => registry.table(['lake', 'region7', 'table7']))
benchmark('warehouse/resolve_registered_namespace', () => registry.namespace('lake.region7'))

// Listing a folder catalog: one `readdir` per level, against `node:fs`.
benchmark('warehouse/list_namespaces', () => [...market.namespaces().keys()])
benchmark('node:fs/list_namespaces', () => fs.readdirSync(root, { withFileTypes: true }))
benchmark('warehouse/list_tables', () => [...market.namespaces().get('region0').tables().keys()])
benchmark('node:fs/list_tables', () =>
  fs.readdirSync(path.join(root, 'region0'), { withFileTypes: true }),
)
benchmark('warehouse/resolve_folder_table', () => market.table('region7.table7'))
benchmark('warehouse/children', () => [...market.children()])

// Records: the handle a table is, against a local handle on the same file.
// A record read is thousands of times the work of a path lookup, so it
// counts in hundreds rather than tens of thousands.
const recordIterations = Math.max(1, Math.floor(iterations / 100))
function recordBenchmark(name, operation) {
  for (let index = 0; index < Math.min(recordIterations, 10); index += 1) operation()
  const started = performance.now()
  for (let index = 0; index < recordIterations; index += 1) operation()
  const elapsed = performance.now() - started
  const rate = Math.round((recordIterations * 1_000) / elapsed)
  console.log(`${name}: ${rate.toLocaleString('en-US')} operations/second`)
}

const registered = registry.table('lake.region0.table0')
recordBenchmark('warehouse/read_records', () =>
  IOBase.from(registered).readArrowReader().intoTable(),
)
recordBenchmark('local/read_records', () => new IOBase(leaf).readArrowReader().intoTable())
recordBenchmark('warehouse/field', () => market.table('region0.table0').field())

fs.rmSync(root, { recursive: true, force: true })
