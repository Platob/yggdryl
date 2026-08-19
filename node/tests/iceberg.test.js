'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')

const arrow = require('apache-arrow')

const { DataType, Field, IOBase, Value, fields, iceberg } = require('yggdryl')

function scratch() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-iceberg-'))
}

// An Iceberg schema is a root Field whose columns carry field identifiers, so
// every table here starts from a numbered copy of one.
function schema() {
  return iceberg.assignFieldIds(
    fields.struct('row', [Field.from('id: int64'), Field.from('venue: utf8')], {
      nullable: false,
    }),
  )
}

function rows(ids, venues) {
  return new arrow.Table({
    id: arrow.vectorFromArray(ids, new arrow.Int64()),
    venue: arrow.vectorFromArray(venues, new arrow.Utf8()),
  })
}

test('numbering a schema is a copy, and the numbers are Arrow field ids', () => {
  const plain = fields.struct('row', [Field.from('id: int64'), Field.from('venue: utf8')], {
    nullable: false,
  })
  const numbered = schema()

  assert.equal(plain.dataType.at(0).parquetFieldId, null)
  assert.equal(numbered.dataType.at(0).parquetFieldId, 1)
  assert.equal(numbered.dataType.at(1).parquetFieldId, 2)
  assert.equal(numbered.dataType.at(0).get('PARQUET:field_id'), '1')

  // Numbering starts where a caller says, so an evolution never reuses one.
  const later = iceberg.assignFieldIds(plain, 10)
  assert.equal(later.dataType.at(0).parquetFieldId, 10)
})

test('creating a table numbers a plain schema itself, partitioning included', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const plain = fields.struct('row', [Field.from('id: int64'), Field.from('venue: utf8')], {
    nullable: false,
  })
  const table = iceberg.Table.create(path.join(root, 'trades'), plain, ['venue'])

  // Depth-first from 1, and the spec resolved 'venue' against the numbering.
  assert.equal(table.schema.dataType.at(0).parquetFieldId, 1)
  assert.equal(table.schema.dataType.at(1).parquetFieldId, 2)
  assert.equal(table.spec.fields[0].sourceId, 2)

  // The numbered table is a working table.
  table.append(rows([1n, 2n], ['XNAS', 'XNYS']))
  assert.equal(table.scan().toTable().numRows, 2)
})

test('a table is a folder, and a new one has no current snapshot', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const location = path.join(root, 'trades')

  const table = iceberg.Table.create(location, schema(), iceberg.PartitionSpec.unpartitioned())
  assert.ok(table.root.isDir())
  assert.equal(table.schemas.length, 1)
  assert.ok(table.spec.isUnpartitioned())
  assert.equal(table.formatVersion, 2)
  assert.equal(table.version, 1)
  assert.equal(table.metadataFileName, 'v1.metadata.json')
  assert.ok(table.metadataLocation.endsWith('/metadata/v1.metadata.json'))
  assert.ok(table.toString().startsWith('file:///'))

  // Everything is a child of the one handle the table was built from.
  const handle = new IOBase(location)
  assert.deepEqual(
    [...handle.joinpath('metadata').iterdir()].map((child) => child.name).sort(),
    ['v1.metadata.json', 'version-hint.text'],
  )

  // A table that has never been written to reads as no rows, not as a failure.
  assert.equal(table.currentSnapshot, null)
  assert.equal(table.snapshots.length, 0)
  assert.equal(table.manifests().length, 0)
  assert.equal(table.scan().toTable().numRows, 0)
})

test('an append commits a snapshot, one data file per partition', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const declared = schema()
  // A list of column names is the short spelling of an identity spec.
  const table = iceberg.Table.create(path.join(root, 'trades'), declared, ['venue'])
  table.append(rows([1n, 2n, 3n], ['XNAS', 'XNYS', 'XNAS']))

  const snapshot = table.currentSnapshot
  assert.equal(snapshot.operation, 'append')
  assert.equal(typeof snapshot.snapshotId, 'bigint')
  assert.equal(snapshot.summary['added-records'], '3')
  assert.equal(table.snapshots.length, 1)

  const [manifest] = table.manifests()
  assert.equal(manifest.content, 'data')
  assert.equal(manifest.addedFilesCount, 2)
  assert.equal(manifest.addedRowsCount, 3)
  assert.equal(manifest.addedSnapshotId, snapshot.snapshotId)

  const files = table.dataFiles().sort((left, right) =>
    left.filePath.localeCompare(right.filePath),
  )
  assert.equal(files.length, 2)
  assert.equal(files[0].fileFormat, 'PARQUET')
  assert.deepEqual(files[0].partitionNames, ['venue'])
  // The manifest is the authority on a partition value, not the directory name.
  assert.deepEqual(
    files.map((file) => file.partition.map((value) => value.asJs())),
    [['XNAS'], ['XNYS']],
  )
  assert.equal(files[0].recordCount + files[1].recordCount, 3)
  assert.ok(files[0].valueCounts.some((entry) => entry.fieldId === 1))
  assert.ok(files[0].toString().includes('venue=XNAS'))

  // The Hive layout is a real one: a directory per partition value.
  const data = new IOBase(path.join(root, 'trades', 'data'))
  assert.deepEqual(
    [...data.iterdir()].map((child) => child.name).sort(),
    ['venue=XNAS', 'venue=XNYS'],
  )

  assert.equal(table.scan().toTable().numRows, 3)
})

test('a scan pushes columns down and casts what each file gives back', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const declared = schema()
  const table = iceberg.Table.create(path.join(root, 'trades'), declared)
  table.append(rows([1n, 2n], ['XNAS', 'XNYS']))

  const wanted = fields.struct('row', [declared.dataType.at(0)], { nullable: false })
  const scanned = table.scan(wanted).toTable()
  assert.equal(scanned.numCols, 1)
  assert.equal(scanned.numRows, 2)
  assert.deepEqual(scanned.getChild('id').toArray(), BigInt64Array.from([1n, 2n]))
})

test('an overwrite keeps the previous snapshot readable', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const table = iceberg.Table.create(path.join(root, 'trades'), schema())
  table.append(rows([1n, 2n], ['XNAS', 'XNYS']))
  const first = table.currentSnapshot.snapshotId

  table.overwrite(rows([3n], ['XNAS']))
  assert.equal(table.currentSnapshot.operation, 'overwrite')
  assert.equal(table.scan().toTable().numRows, 1)

  // Nothing was mutated in place: the snapshot before it is still recorded.
  assert.equal(table.snapshots.length, 2)
  assert.ok(table.snapshots.some((snapshot) => snapshot.snapshotId === first))
})

test('a schema evolves, and files written before a column read null for it', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const table = iceberg.Table.create(path.join(root, 'trades'), schema())
  table.append(rows([1n], ['XNAS']))

  const evolved = iceberg.assignFieldIds(
    fields.struct(
      'row',
      [Field.from('id: int64'), Field.from('venue: utf8'), Field.from('price: float64')],
      { nullable: false },
    ),
  )
  assert.equal(table.evolveSchema(evolved), 1)
  assert.equal(table.schema.dataType.length, 3)

  const scanned = table.scan().toTable()
  assert.equal(scanned.numRows, 1)
  assert.equal(scanned.getChild('price').get(0), null)
})

test('a table is found again with no catalog in between', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const location = path.join(root, 'trades')

  const created = iceberg.Table.create(location, schema())
  created.append(rows([1n, 2n], ['XNAS', 'XNYS']))
  const uuid = created.tableUuid

  const reopened = iceberg.Table.open(location)
  assert.equal(reopened.tableUuid, uuid)
  assert.equal(reopened.version, created.version)
  assert.equal(reopened.scan().toTable().numRows, 2)

  // Opening what is there and creating what is not is one call.
  const either = iceberg.Table.openOrCreate(location, schema())
  assert.equal(either.tableUuid, uuid)
  assert.throws(() => iceberg.Table.open(path.join(root, 'absent')), /metadata/)
})

test('a transform that cannot place a row is refused by name', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const declared = schema()
  const spec = iceberg.PartitionSpec.identity(declared, ['venue'], 1)
  assert.equal(spec.specId, 1)
  assert.deepEqual(spec.fields, [
    { sourceId: 2, fieldId: 1000, name: 'venue', transform: 'identity' },
  ])
  assert.ok(!spec.isUnpartitioned())
  assert.ok(iceberg.PartitionSpec.unpartitioned().isUnpartitioned())

  assert.throws(
    () => iceberg.PartitionSpec.identity(declared, ['nowhere'], 1),
    /nowhere/,
  )
})

test('a catalog maps a dotted name onto folders and creates on first write', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const catalog = new iceberg.Catalog(root)
  assert.equal(catalog.hasTable('nyc.taxis'), false)

  // The first append creates the table from the reader's own schema.
  const table = catalog.append('nyc.taxis', rows([1n, 2n], ['XNAS', 'XNYS']))
  assert.ok(catalog.hasTable('nyc.taxis'))
  assert.equal(table.schema.name, 'row')
  assert.equal(table.schema.dataType.at(0).parquetFieldId, 1)
  assert.equal(table.scan().toTable().numRows, 2)

  // The dotted name is the folder nyc/taxis, one level per dot.
  const handle = new IOBase(path.join(root, 'nyc', 'taxis'))
  assert.ok(handle.isDir())
  assert.deepEqual(catalog.listNamespaces(), ['nyc'])
  assert.deepEqual(catalog.listTables('nyc'), ['nyc.taxis'])

  // A second append accumulates rather than replacing.
  const again = catalog.append('nyc.taxis', rows([3n], ['XASE']))
  assert.equal(again.scan().toTable().numRows, 3)
  assert.equal(catalog.table('nyc.taxis').scan().toTable().numRows, 3)

  // A schema is a Field, a string expression, or an array of child Fields.
  const rides = catalog.createTable('nyc.rides', [
    Field.from('id: int64'),
    Field.from('city: utf8'),
  ])
  assert.equal(rides.schema.name, 'row')
  assert.deepEqual(
    Array.from(rides.schema.dataType, (child) => child.name),
    ['id', 'city'],
  )
  catalog.createTable('nyc.zones', 'row: struct<id int64, zone utf8> not null')
  assert.deepEqual(catalog.listTables('nyc'), ['nyc.rides', 'nyc.taxis', 'nyc.zones'])

  // Creating what exists is refused; opening-or-creating is one call.
  assert.throws(() => catalog.createTable('nyc.taxis', schema()), /nyc\.taxis/)
  const either = catalog.openOrCreateTable('nyc.taxis', schema())
  assert.equal(either.tableUuid, table.tableUuid)

  // An overwrite through the catalog keeps the previous snapshot readable.
  const replaced = catalog.overwrite('nyc.taxis', rows([9n], ['XNAS']))
  assert.equal(replaced.scan().toTable().numRows, 1)
  assert.equal(replaced.snapshots.length, 3)
})

test('scanAt reads a retained snapshot after an overwrite', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const table = iceberg.Table.create(path.join(root, 'trades'), schema())
  table.append(rows([1n, 2n], ['XNAS', 'XNYS']))
  const first = table.currentSnapshot.snapshotId
  table.overwrite(rows([3n], ['XASE']))
  assert.equal(table.scan().toTable().numRows, 1)

  // The overwritten snapshot is still a complete table.
  const past = table.scanAt(first).toTable()
  assert.equal(past.numRows, 2)
  assert.deepEqual(past.getChild('id').toArray(), BigInt64Array.from([1n, 2n]))

  // Filters are the pairs childrenWhere takes, and a schema keeps the
  // columns it names, exactly as on scan.
  const wanted = fields.struct('row', [Field.from('id: int64')], { nullable: false })
  const filtered = table.scanAt(first, { venue: 'XNAS' }, wanted).toTable()
  assert.equal(filtered.numCols, 1)
  assert.equal(filtered.numRows, 1)
  assert.deepEqual(filtered.getChild('id').toArray(), BigInt64Array.from([1n]))

  // A snapshot id crosses as a bigint or as an exact number, and an id the
  // table does not retain is refused naming the ids it does.
  assert.throws(() => table.scanAt(7), new RegExp(`got 7; the table retains \\[.*${first}`))
  assert.throws(() => table.scanAt(2 ** 60), /at most 2\^53/)
})

test('a schema evolves through one recorded chain, committed once', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const declared = iceberg.assignFieldIds(
    fields.struct('row', [Field.from('id: int32'), Field.from('venue: utf8')], {
      nullable: false,
    }),
  )
  const table = iceberg.Table.create(path.join(root, 'trades'), declared)
  table.append(
    new arrow.Table({
      id: arrow.vectorFromArray([1], new arrow.Int32()),
      venue: arrow.vectorFromArray(['XNAS'], new arrow.Utf8()),
    }),
  )
  const version = table.version

  const schemaId = table
    .updateSchema()
    .addColumn('', 'price: float64')
    .updateType('id', DataType.from('int64'))
    .renameColumn('venue', 'market')
    .commit()

  // One chain is one commit: one new schema, one new metadata document.
  assert.equal(schemaId, 1)
  assert.equal(table.version, version + 1)
  assert.equal(table.schemas.length, 2)

  const evolved = table.schema
  assert.deepEqual(
    Array.from(evolved.dataType, (child) => child.name),
    ['id', 'market', 'price'],
  )
  assert.equal(evolved.dataType.at(0).dataType.id, 'int64')
  // The renamed column keeps its identifier; the added one is numbered fresh.
  assert.equal(evolved.dataType.at(1).parquetFieldId, 2)
  assert.equal(evolved.dataType.at(2).parquetFieldId, 3)

  // The rows written under the old schema are preserved and read as the new
  // shape: the promoted column widens, the added column reads null.
  const scanned = table.scan().toTable()
  assert.equal(scanned.numRows, 1)
  assert.deepEqual(scanned.getChild('id').toArray(), BigInt64Array.from([1n]))
  assert.equal(scanned.getChild('price').get(0), null)

  // An illegal promotion is refused at commit time with the core message.
  assert.throws(
    () => table.updateSchema().updateType('id', 'int32').commit(),
    /expected an Iceberg-legal promotion, got int64 to int32/,
  )
  // A failed commit is a commit that never happened.
  assert.equal(table.version, version + 1)
})

test('updateProperties commits once, and nothing when there is nothing', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const location = path.join(root, 'trades')

  const table = iceberg.Table.create(location, schema())
  const version = table.version

  table.updateProperties({ 'commit.retry.num-retries': '4' })
  assert.equal(table.properties['commit.retry.num-retries'], '4')
  assert.equal(table.version, version + 1)

  // A Map updates and an array removes, in that order, as one commit.
  table.updateProperties(new Map([['write.target-file-size-bytes', '1024']]), [
    'commit.retry.num-retries',
  ])
  assert.equal(table.properties['commit.retry.num-retries'], undefined)
  assert.equal(table.targetFileSize, 1024)
  assert.equal(table.version, version + 2)

  // Nothing to change commits nothing at all.
  table.updateProperties()
  table.updateProperties({}, [])
  assert.equal(table.version, version + 2)

  // The properties are in the document, not in this wrapper.
  const reopened = iceberg.Table.open(location)
  assert.equal(reopened.properties['write.target-file-size-bytes'], '1024')
  assert.equal(iceberg.Table.open(location).targetFileSize, 1024)
})

test('compact merges undersized files and reports what it rewrote', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const table = iceberg.Table.create(path.join(root, 'trades'), schema())
  table.append(rows([1n, 2n], ['XNAS', 'XNYS']))
  table.append(rows([3n], ['XASE']))
  const before = table.currentSnapshot.snapshotId

  // The default target is Iceberg's own 512 MiB.
  assert.equal(table.targetFileSize, 512 * 1024 * 1024)

  const sizes = table.dataFiles().map((file) => file.fileSizeInBytes)
  assert.equal(table.inspectFiles().toTable().numRows, 2)

  const compaction = table.compact()
  assert.equal(compaction.filesBefore, 2)
  assert.equal(compaction.filesAfter, 1)
  assert.equal(
    compaction.bytesRewritten,
    sizes.reduce((total, size) => total + size, 0),
  )
  assert.equal(table.currentSnapshot.operation, 'replace')
  assert.equal(table.inspectFiles().toTable().numRows, 1)
  assert.equal(table.scan().toTable().numRows, 3)

  // The pre-compaction snapshot is retained and reads exactly as it did.
  assert.equal(table.scanAt(before).toTable().numRows, 3)

  // A table with nothing to compact commits nothing and reports zeros.
  const version = table.version
  assert.deepEqual(table.compact(), { filesBefore: 0, filesAfter: 0, bytesRewritten: 0 })
  assert.equal(table.version, version)

  // The inspection tables carry the history under PyIceberg's column names.
  const history = table.inspectHistory().toTable()
  assert.equal(history.numRows, 3)
  assert.deepEqual(
    history.schema.fields.map((field) => field.name),
    ['made_current_at', 'snapshot_id', 'parent_id', 'is_current_ancestor'],
  )
  const snapshots = table.inspectSnapshots().toTable()
  assert.equal(snapshots.numRows, 3)
  assert.equal(snapshots.getChild('operation').get(2), 'replace')
})

test('canPromote answers the promotion list in both directions', () => {
  // Legal promotions pass and return nothing.
  iceberg.canPromote('int32', 'int64')
  iceberg.canPromote('float32', 'float64')
  iceberg.canPromote(DataType.from('decimal64(10, 2)'), 'decimal128(20, 2)')
  iceberg.canPromote('utf8', 'utf8')

  // The reverse direction, and everything else, is refused naming both sides.
  assert.throws(
    () => iceberg.canPromote('int64', 'int32'),
    /expected an Iceberg-legal promotion, got int64 to int32/,
  )
  assert.throws(() => iceberg.canPromote('float64', 'float32'), /float64 to float32/)
  assert.throws(() => iceberg.canPromote('utf8', 'int64'), /utf8 to int64/)
})

test('a ref names a snapshot, and a missing one names the refs the table has', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const table = iceberg.Table.create(path.join(root, 'trades'), schema())
  table.append(rows([1n], ['XNAS']))

  // Every commit moves the main branch, so a fresh append is reachable by ref.
  const main = table.snapshotByRef('main')
  assert.equal(main.snapshotId, table.currentSnapshot.snapshotId)

  assert.throws(
    () => table.snapshotByRef('nightly'),
    /expected a branch or tag this table has, got "nightly"; it has \[main\]/,
  )
})

test('a schema is a document in both directions', () => {
  const declared = schema()
  const document = iceberg.schemaToJson(declared)
  assert.deepEqual(document.asJs(), {
    type: 'struct',
    fields: [
      { id: 1, name: 'id', required: false, type: 'long' },
      { id: 2, name: 'venue', required: false, type: 'string' },
    ],
  })

  const read = iceberg.schemaFromJson('row', document)
  assert.ok(read.equals(declared))

  // A document another catalog handed over reads the same way, as the native
  // value or as the plain object a JSON decoder produced.
  const foreign = {
    type: 'struct',
    'schema-id': 0,
    fields: [{ id: 1, name: 'id', required: true, type: 'long' }],
  }
  assert.ok(
    iceberg
      .schemaFromJson('trade', Value.fromJs(foreign))
      .equals(iceberg.schemaFromJson('trade', foreign)),
  )
  const imported = iceberg.schemaFromJson('trade', foreign)
  assert.equal(imported.name, 'trade')
  assert.equal(imported.dataType.length, 1)
  assert.equal(imported.dataType.at(0).nullable, false)
})

test('a namespace is the map-like half of the catalog', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const catalog = new iceberg.Catalog(root)
  const analytics = catalog.namespace('analytics')
  assert.equal(analytics.name, 'analytics')

  // Setting a schema gets or creates; setting again is the same table.
  const schema = new Field('row', 'struct<id: int64, venue: utf8>', false)
  analytics.set('trades', schema)
  analytics.set('trades', schema)
  assert.ok(analytics.has('trades'))
  // `tables` is the collection itself, so the name list comes off the view.
  assert.deepEqual(analytics.tables.names(), ['trades'])

  // Setting rows replaces the table's rows, creating a table the namespace
  // never had from the rows' own schema.
  analytics.set('quotes', rows([1n, 2n], ['XNAS', 'XNYS']))
  assert.equal(analytics.get('quotes').scan().toTable().numRows, 2)
  assert.deepEqual(analytics.tables.names().sort(), ['quotes', 'trades'])
  assert.deepEqual(catalog.listNamespaces(), ['analytics'])
})

// The options value is a recording of what a caller set, never a snapshot of
// every field, so each test below asks both halves: what it named, and what it
// left alone.
test('an options value answers the fields it was given and defaults the rest', () => {
  const untouched = new iceberg.IcebergOptions()
  assert.equal(untouched.commitRetries, 4)
  assert.equal(untouched.commitMinBackoffMs, 100)
  assert.equal(untouched.commitMaxBackoffMs, 60_000)
  assert.equal(untouched.targetFileSize, 512 * 1024 * 1024)
  assert.equal(untouched.readParallelMinFiles, 16)
  assert.equal(untouched.readParallelMinFileSize, 4 * 1024 * 1024)
  assert.equal(untouched.dataFormat, 'PARQUET')
  // Nothing compacts on its own until a cadence says so.
  assert.equal(untouched.compactAfterCommits, null)
  // Read parallelism defaults to what the host offers, kept inside 1..=8.
  assert.ok(untouched.readParallelism >= 1 && untouched.readParallelism <= 8)

  const given = new iceberg.IcebergOptions({
    commitRetries: 9,
    commitMinBackoffMs: 5,
    commitMaxBackoffMs: 50,
    targetFileSize: 4096,
    readParallelism: 2,
    readParallelMinFiles: 3,
    readParallelMinFileSize: 1024,
    compactAfterCommits: 7,
    dataFormat: 'avro',
  })
  assert.equal(given.commitRetries, 9)
  assert.equal(given.commitMinBackoffMs, 5)
  assert.equal(given.commitMaxBackoffMs, 50)
  assert.equal(given.targetFileSize, 4096)
  assert.equal(given.readParallelism, 2)
  assert.equal(given.readParallelMinFiles, 3)
  assert.equal(given.readParallelMinFileSize, 1024)
  assert.equal(given.compactAfterCommits, 7)
  assert.equal(given.dataFormat, 'AVRO')

  // Every field is a setter too, and one the object never named still answers
  // its default rather than whatever a neighbouring field was set to.
  const partial = new iceberg.IcebergOptions({ commitRetries: 1 })
  assert.equal(partial.commitRetries, 1)
  assert.equal(partial.targetFileSize, 512 * 1024 * 1024)
  partial.targetFileSize = 8192
  partial.compactAfterCommits = 2
  assert.equal(partial.targetFileSize, 8192)
  assert.equal(partial.compactAfterCommits, 2)
  assert.equal(partial.commitMinBackoffMs, 100)
})

test('a data format is named in any case and refused when no writer has it', () => {
  const options = new iceberg.IcebergOptions({ dataFormat: 'avro' })
  assert.equal(options.dataFormat, 'AVRO')
  options.dataFormat = 'PaRqUeT'
  assert.equal(options.dataFormat, 'PARQUET')
  assert.equal(new iceberg.IcebergOptions({ dataFormat: 'AVRO' }).dataFormat, 'AVRO')

  assert.throws(
    () => new iceberg.IcebergOptions({ dataFormat: 'csv' }),
    /expected an Iceberg file format of PARQUET, AVRO, or ORC, got "CSV"/,
  )
  assert.throws(() => {
    options.dataFormat = 'csv'
  }, /got "CSV"/)
  // A refused write leaves the field holding what it held.
  assert.equal(options.dataFormat, 'PARQUET')
})

test('a zero file size and a zero read parallelism are refused by property name', () => {
  // A target no file can meet would roll one file per batch forever, and a
  // reader count of zero would read nothing, so both are refused where they
  // are set rather than obeyed later.
  assert.throws(
    () => new iceberg.IcebergOptions({ targetFileSize: 0 }),
    /write\.target-file-size-bytes.*expected a positive byte count, got 0/,
  )
  assert.throws(
    () => new iceberg.IcebergOptions({ readParallelism: 0 }),
    /read\.parallelism.*expected at least one reader thread, got 0/,
  )

  const options = new iceberg.IcebergOptions({ targetFileSize: 4096, readParallelism: 2 })
  assert.throws(() => {
    options.targetFileSize = 0
  }, /expected a positive byte count, got 0/)
  assert.throws(() => {
    options.readParallelism = 0
  }, /expected at least one reader thread, got 0/)
  assert.equal(options.targetFileSize, 4096)
  assert.equal(options.readParallelism, 2)
})

test('a per-call data format writes AVRO files beside the PARQUET ones', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const table = iceberg.Table.create(path.join(root, 'trades'), schema())
  table.append(rows([1n, 2n], ['XNAS', 'XNYS']))
  table.append(rows([3n], ['XASE']), new iceberg.IcebergOptions({ dataFormat: 'avro' }))

  // The format is the assertion. A wrapper that dropped the options argument
  // would still commit, still return, and still scan - and write Parquet
  // twice, which nothing but the manifest entry would reveal.
  assert.deepEqual(
    table.dataFiles().map((file) => file.fileFormat).sort(),
    ['AVRO', 'PARQUET'],
  )

  // A scan decodes each file as the format its manifest entry records, so a
  // table of two formats still reads as one shape.
  const scanned = table.scan().toTable()
  assert.equal(scanned.numRows, 3)
  assert.deepEqual(
    [...scanned.getChild('id')].sort((left, right) => Number(left - right)),
    [1n, 2n, 3n],
  )
  // The call configured itself alone: nothing was stored on the handle.
  assert.equal(table.options().dataFormat, 'PARQUET')
})

test('setOptions is what later calls resolve, and a per-call option outlives only its call', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const table = iceberg.Table.create(path.join(root, 'trades'), schema())
  table.setOptions(new iceberg.IcebergOptions({ targetFileSize: 4096, dataFormat: 'avro' }))
  assert.equal(table.options().dataFormat, 'AVRO')
  // The override shadows the table property the getter would otherwise read.
  assert.equal(table.targetFileSize, 4096)

  table.append(rows([1n], ['XNAS']))
  assert.deepEqual(table.dataFiles().map((file) => file.fileFormat), ['AVRO'])

  table.append(rows([2n], ['XNYS']), new iceberg.IcebergOptions({ dataFormat: 'parquet' }))
  assert.deepEqual(
    table.dataFiles().map((file) => file.fileFormat).sort(),
    ['AVRO', 'PARQUET'],
  )

  // The override is put back whatever the call did, and a field the per-call
  // value never named was never in question.
  assert.equal(table.options().dataFormat, 'AVRO')
  assert.equal(table.options().targetFileSize, 4096)
})

// Testing the per-call option on `append` alone is what let three methods ship
// accepting an options argument they never declared: the JS wrapper forwarded
// it, napi discarded the extra, and every one of them committed, returned, and
// scanned exactly as if it had worked. One case per write is the only shape of
// this test that would have caught it.
test('every write that takes a per-call data format actually writes it', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const avro = () => new iceberg.IcebergOptions({ dataFormat: 'avro' })
  const formats = (table) => table.dataFiles().map((file) => file.fileFormat).sort()

  const overwritten = iceberg.Table.create(path.join(root, 'ow'), schema(), ['venue'])
  overwritten.append(rows([1n, 2n], ['XNAS', 'XNYS']))
  overwritten.overwriteWhere({ venue: 'XNAS' }, rows([9n], ['XNAS']), avro())
  // XNYS is carried forward as the PARQUET file it already was; only the
  // partition this call rewrote is AVRO.
  assert.deepEqual(formats(overwritten), ['AVRO', 'PARQUET'])

  const merged = iceberg.Table.create(path.join(root, 'mg'), schema())
  merged.append(rows([1n, 2n], ['XNAS', 'XNYS']))
  merged.merge(rows([2n, 3n], ['XLON', 'XASE']), ['id'], true, avro())
  assert.deepEqual(formats(merged), ['AVRO'])
  assert.equal(merged.scan().toTable().numRows, 3)

  const mergedWhere = iceberg.Table.create(path.join(root, 'mw'), schema(), ['venue'])
  mergedWhere.append(rows([1n, 2n], ['XNAS', 'XNYS']))
  mergedWhere.mergeWhere({ venue: 'XNAS' }, rows([1n], ['XNAS']), ['id'], true, avro())
  assert.deepEqual(formats(mergedWhere), ['AVRO', 'PARQUET'])

  const replaced = iceberg.Table.create(path.join(root, 'ov'), schema())
  replaced.append(rows([1n], ['XNAS']))
  replaced.overwrite(rows([2n], ['XNYS']), avro())
  assert.deepEqual(formats(replaced), ['AVRO'])

  // None of them stored anything on the handle.
  for (const table of [overwritten, merged, mergedWhere, replaced]) {
    assert.equal(table.options().dataFormat, 'PARQUET')
  }
})

test('the catalog write shorthands honour a per-call data format', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  // `catalog.append(name, rows)` and `namespace.tables.append(name, rows)` are
  // two spellings of one operation. One of them honoured the option and the
  // other wrote PARQUET and returned a table, which is the divergence a caller
  // has no way to see without opening the manifest.
  const catalog = new iceberg.Catalog(root)
  catalog.createTable('sales.orders', schema())
  catalog.append('sales.orders', rows([1n], ['XNAS']), new iceberg.IcebergOptions({ dataFormat: 'avro' }))
  assert.deepEqual(catalog.table('sales.orders').dataFiles().map((f) => f.fileFormat), ['AVRO'])

  catalog.overwrite('sales.orders', rows([2n], ['XNYS']), new iceberg.IcebergOptions({ dataFormat: 'avro' }))
  assert.deepEqual(catalog.table('sales.orders').dataFiles().map((f) => f.fileFormat), ['AVRO'])

  // The view spelling agrees, which is the whole point of there being two.
  catalog.namespaces.get('sales').tables.append('orders', rows([3n], ['XLON']))
  assert.deepEqual(
    catalog.table('sales.orders').dataFiles().map((f) => f.fileFormat).sort(),
    ['AVRO', 'PARQUET'],
  )
})

test('the filtered reads take the per-call options the plain scan does', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const table = iceberg.Table.create(path.join(root, 'trades'), schema(), ['venue'])
  table.append(rows([1n, 2n], ['XNAS', 'XNYS']))
  table.createTag('release', table.currentSnapshot.snapshotId)

  // The option has to reach the reader for the call to be honest about
  // accepting one; the rows it returns are what says it arrived intact.
  const before = table.options().readParallelism
  const options = new iceberg.IcebergOptions({ readParallelism: before + 1 })
  assert.equal(table.scanWhere({ venue: 'XNAS' }, null, options).toTable().numRows, 1)
  assert.equal(table.scanRef('release', null, null, options).toTable().numRows, 2)
  // Configuring one call configures only that call.
  assert.equal(table.options().readParallelism, before)
})

test('a tag and a branch name a snapshot, and removing one reports what it held', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const table = iceberg.Table.create(path.join(root, 'trades'), schema(), ['venue'])
  table.append(rows([1n, 2n], ['XNAS', 'XNYS']))
  const first = table.currentSnapshot.snapshotId
  table.append(rows([3n], ['XASE']))
  const second = table.currentSnapshot.snapshotId

  table.createTag('nightly', first)
  table.createBranch('audit', first)
  assert.equal(table.snapshotByRef('nightly').snapshotId, first)

  // A ref reads as an ordinary scan of the snapshot it points at, filters and
  // projection included.
  assert.equal(table.scanRef('audit').toTable().numRows, 2)
  assert.equal(table.scanRef('audit', { venue: 'XNAS' }).toTable().numRows, 1)

  const removed = table.removeRef('nightly')
  assert.equal(removed.snapshotId, first)
  assert.equal(removed.kind, 'tag')

  // Dropping a ref that was never there is a typo far more often than it is a
  // no-op, so it is refused naming the refs the table does have.
  assert.throws(() => table.removeRef('nightly'), /got "nightly"; it has \[main, audit\]/)
  assert.equal(table.snapshotByRef('audit').snapshotId, first)
  assert.notEqual(first, second)
})

test('fastForward moves a branch onto a descendant and refuses to walk back', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const table = iceberg.Table.create(path.join(root, 'trades'), schema())
  table.append(rows([1n], ['XNAS']))
  const first = table.currentSnapshot.snapshotId
  table.append(rows([2n], ['XNYS']))
  const second = table.currentSnapshot.snapshotId

  table.createBranch('audit', first)
  assert.equal(table.scanRef('audit').toTable().numRows, 1)
  table.fastForward('audit', second)
  assert.equal(table.snapshotByRef('audit').snapshotId, second)
  assert.equal(table.scanRef('audit').toTable().numRows, 2)

  // Walking a branch backwards would drop the commits between, which is the
  // one thing a fast-forward is defined not to do.
  assert.throws(
    () => table.fastForward('audit', first),
    new RegExp(`expected ${first} to descend from ${second}`),
  )
})

test('a plan counts what a scan would read without reading any of it', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const table = iceberg.Table.create(path.join(root, 'trades'), schema(), ['venue'])
  table.append(rows([1n, 2n, 3n], ['XNAS', 'XNYS', 'XNAS']))
  const first = table.currentSnapshot.snapshotId
  table.append(rows([4n], ['XASE']))

  const everything = table.plan()
  assert.equal(everything.recordCount, 4)
  assert.equal(everything.filesPlanned, table.dataFiles().length)
  assert.equal(everything.filesSkipped, 0)
  assert.equal(everything.manifestsRead, 2)
  assert.equal(everything.manifestsSkipped, 0)

  // Pruning is a number rather than a promise: one manifest was ruled out
  // whole by the manifest list's summaries, one file inside the other by its
  // partition tuple, and the rows left agree with what a scan yields.
  const matching = table.plan({ venue: 'XNAS' })
  assert.equal(matching.filesPlanned, 1)
  assert.equal(matching.filesSkipped, 1)
  assert.equal(matching.manifestsRead, 1)
  assert.equal(matching.manifestsSkipped, 1)
  assert.equal(matching.recordCount, table.scanWhere({ venue: 'XNAS' }).toTable().numRows)

  // Time travel plans over the snapshot's own manifest list, so a filtered
  // read of history skips what a filtered read of the present skips.
  const past = table.planAt(first, { venue: 'XNAS' })
  assert.equal(past.recordCount, 2)
  assert.equal(past.filesPlanned, 1)
  assert.equal(past.filesSkipped, 1)

  // A partition value the table never held reaches no manifest at all: the
  // cheapest of the three levels answered the whole question.
  const absent = table.plan({ venue: 'XLON' })
  assert.equal(absent.recordCount, 0)
  assert.equal(absent.filesPlanned, 0)
  assert.equal(absent.filesSkipped, 0)
  assert.equal(absent.manifestsRead, 0)
  assert.equal(absent.manifestsSkipped, 2)
})

test('manifestsAt answers for a retained snapshot what manifests answers for now', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const table = iceberg.Table.create(path.join(root, 'trades'), schema())
  table.append(rows([1n], ['XNAS']))
  const first = table.currentSnapshot.snapshotId
  table.append(rows([2n], ['XNYS']))

  assert.equal(table.manifests().length, 2)
  const past = table.manifestsAt(first)
  assert.equal(past.length, 1)
  assert.equal(past[0].addedSnapshotId, first)
  assert.equal(past[0].addedRowsCount, 1)

  // An id the table does not retain is refused naming the ids it does.
  assert.throws(() => table.manifestsAt(7), new RegExp(`got 7; the table retains \\[.*${first}`))
})

test('scanWhere reads only the partition it names and projects as scan does', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const table = iceberg.Table.create(path.join(root, 'trades'), schema(), ['venue'])
  table.append(rows([1n, 2n, 3n], ['XNAS', 'XNYS', 'XNAS']))

  const matched = table.scanWhere({ venue: 'XNAS' }).toTable()
  assert.equal(matched.numRows, 2)
  assert.deepEqual([...matched.getChild('venue')], ['XNAS', 'XNAS'])

  // The projection sits after the filters, and keeps the columns it names.
  const projected = table.scanWhere({ venue: 'XNAS' }, 'row: struct<id int64> not null').toTable()
  assert.equal(projected.numCols, 1)
  assert.deepEqual([...projected.getChild('id')], [1n, 3n])

  // Filters also spell as the `{column, value}` entries a manifest reports.
  assert.equal(
    table.scanWhere([{ column: 'venue', value: 'XNYS' }]).toTable().numRows,
    1,
  )

  // A value nothing was ever written under is no rows, with the shape intact.
  const none = table.scanWhere({ venue: 'XLON' }).toTable()
  assert.equal(none.numRows, 0)
  assert.equal(none.numCols, 2)
})

test('overwriteWhere replaces one partition and carries the others as they were', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const table = iceberg.Table.create(path.join(root, 'trades'), schema(), ['venue'])
  table.append(rows([1n, 2n, 3n], ['XNAS', 'XNYS', 'XNAS']))
  const first = table.currentSnapshot.snapshotId
  const kept = table
    .dataFiles()
    .filter((file) => file.partition[0].asJs() === 'XNYS')
    .map((file) => [file.filePath, file.recordCount, file.fileSizeInBytes])
  assert.equal(kept.length, 1)

  table.overwriteWhere({ venue: 'XNAS' }, rows([9n], ['XNAS']))

  // A file the filters exclude is carried into the new snapshot exactly as it
  // was - same location, same statistics - rather than read and written again.
  assert.deepEqual(
    table
      .dataFiles()
      .filter((file) => file.partition[0].asJs() === 'XNYS')
      .map((file) => [file.filePath, file.recordCount, file.fileSizeInBytes]),
    kept,
  )
  assert.deepEqual(
    [...table.scan().toTable().getChild('id')].sort((left, right) => Number(left - right)),
    [2n, 9n],
  )
  // Only the current pointer moved: the snapshot before it is still whole.
  assert.equal(table.scanAt(first).toTable().numRows, 3)
})

test('merge updates the rows whose key is stored and appends the rest', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const table = iceberg.Table.create(path.join(root, 'trades'), schema())
  table.append(rows([1n, 2n], ['XNAS', 'XNYS']))
  table.merge(rows([2n, 3n], ['XASE', 'XLON']), ['id'])

  const scanned = table.scan().toTable()
  const venues = new Map(
    [...scanned.getChild('id')].map((id, index) => [id, scanned.getChild('venue').get(index)]),
  )
  assert.equal(venues.size, 3)
  assert.equal(venues.get(1n), 'XNAS')
  assert.equal(venues.get(2n), 'XASE')
  assert.equal(venues.get(3n), 'XLON')

  // Nothing identifies a row when no column is named, so an empty match key is
  // an overwrite rather than an append that can never find anything.
  table.merge(rows([7n], ['XPAR']), [])
  assert.equal(table.scan().toTable().numRows, 1)
  assert.deepEqual([...table.scan().toTable().getChild('venue')], ['XPAR'])
})

test('mergeWhere narrows a merge to the files its filters admit', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const table = iceberg.Table.create(path.join(root, 'trades'), schema(), ['venue'])
  table.append(rows([1n, 2n, 3n], ['XNAS', 'XNYS', 'XNAS']))
  const kept = table
    .dataFiles()
    .filter((file) => file.partition[0].asJs() === 'XNYS')
    .map((file) => file.filePath)

  table.mergeWhere({ venue: 'XNAS' }, rows([1n, 4n], ['XNAS', 'XNAS']), ['id'])

  // The excluded partition was never a candidate, so its file is the same file.
  assert.deepEqual(
    table
      .dataFiles()
      .filter((file) => file.partition[0].asJs() === 'XNYS')
      .map((file) => file.filePath),
    kept,
  )
  assert.equal(table.scanWhere({ venue: 'XNAS' }).toTable().numRows, 3)
  assert.equal(table.scan().toTable().numRows, 4)
})

test('expireSnapshots drops nothing when the cutoff is older than every snapshot', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const table = iceberg.Table.create(path.join(root, 'trades'), schema())
  table.append(rows([1n], ['XNAS']))
  const first = table.currentSnapshot.snapshotId
  table.append(rows([2n], ['XNYS']))
  assert.equal(table.snapshots.length, 2)

  // One millisecond after the epoch is older than anything a table can hold,
  // so the check runs on a copy and no version is spent.
  const version = table.version
  assert.deepEqual(table.expireSnapshots(1), [])
  assert.equal(table.version, version)
  assert.equal(table.snapshots.length, 2)

  // A cutoff ahead of now drops the ancestors the branch no longer needs.
  assert.deepEqual(table.expireSnapshots(Date.now() + 60_000), [first])
  assert.equal(table.snapshots.length, 1)
  assert.ok(table.version > version)
  // Expiry drops history, never rows: what the current snapshot holds is what
  // it held before the ancestors went away.
  assert.equal(table.scan().toTable().numRows, 2)
})

test('the collection views do no I/O until a question is asked of them', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const catalog = new iceberg.Catalog(root)
  const namespaces = catalog.namespaces
  const orders = catalog.namespace('sales').tables

  // A view describes a question, not an answer: both exist for a warehouse
  // that holds nothing, and building them wrote nothing to it.
  assert.deepEqual(fs.readdirSync(root), [])
  assert.deepEqual(namespaces.names(), [])
  assert.equal(namespaces.size(), 0)
  assert.equal(namespaces.has('sales'), false)
  assert.deepEqual(orders.names(), [])
  assert.equal(orders.size(), 0)
  assert.deepEqual(fs.readdirSync(root), [])

  assert.throws(() => namespaces.get('sales'), /expected a namespace at "sales", got none/)

  // Both views were built before the write and answer storage at call time, so
  // both see it without being rebuilt.
  catalog.append('sales.orders', rows([1n], ['XNAS']))
  assert.deepEqual(namespaces.names(), ['sales'])
  assert.equal(namespaces.size(), 1)
  assert.deepEqual(orders.names(), ['orders'])
  assert.equal(orders.size(), 1)
  assert.equal(orders.has('orders'), true)
  assert.throws(() => orders.get('ledger'), /expected a table at "sales\.ledger", got none/)

  // One spelling chains from the catalog to the rows.
  assert.equal(catalog.namespaces.get('sales').tables.get('orders').scan().toTable().numRows, 1)
  // A namespace is a folder that is not a table, so a table answers false here
  // rather than being reported as a namespace nobody can open.
  assert.equal(catalog.namespaces.get('sales').namespaces.has('orders'), false)
})

test('the tables view creates on first write and takes the same per-call options', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const catalog = new iceberg.Catalog(root)
  const tables = catalog.namespaces.openOrCreate('sales').tables

  const created = tables.create('quotes', schema())
  assert.deepEqual(tables.names(), ['quotes'])
  assert.equal(tables.openOrCreate('quotes', schema()).tableUuid, created.tableUuid)
  assert.throws(
    () => tables.create('quotes', schema()),
    /expected no table at "sales\.quotes", got one/,
  )

  // A write through the view creates the table from the rows' own schema, and
  // forwards the trailing options the way the table's own writes do.
  tables.append('orders', rows([1n], ['XNAS']), new iceberg.IcebergOptions({ dataFormat: 'avro' }))
  tables.append('orders', rows([2n], ['XNYS']))
  assert.deepEqual(
    tables.get('orders').dataFiles().map((file) => file.fileFormat).sort(),
    ['AVRO', 'PARQUET'],
  )

  const replaced = tables.overwrite(
    'orders',
    rows([3n], ['XASE']),
    new iceberg.IcebergOptions({ dataFormat: 'avro' }),
  )
  assert.deepEqual(replaced.dataFiles().map((file) => file.fileFormat), ['AVRO'])
  assert.equal(replaced.scan().toTable().numRows, 1)
  assert.deepEqual(tables.names(), ['orders', 'quotes'])
  assert.equal(tables.size(), 2)
})
