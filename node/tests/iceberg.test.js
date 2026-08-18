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
    handle.joinpath('metadata').iterdir().map((child) => child.name).sort(),
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
    data.iterdir().map((child) => child.name).sort(),
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
