'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')

const arrow = require('apache-arrow')

const { DataType, Field, IOBase, MimeType, Scalar, fields, iceberg } = require('yggdryl')

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

function dataMimeTypes(table) {
  return table.dataFiles().map((file) => file.mimeType.toString()).sort()
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
  assert.equal(table.scan().intoTable().numRows, 2)
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
  assert.match(table.metadataFileName, /^00001-[0-9a-f-]+\.metadata\.json$/)
  assert.ok(table.metadataLocation.endsWith(`/metadata/${table.metadataFileName}`))
  assert.ok(table.toString().startsWith('file:///'))

  // Everything is a child of the one handle the table was built from.
  const handle = new IOBase(location)
  assert.deepEqual(
    [...handle.joinpath('metadata').iterdir()].map((child) => child.name).sort(),
    [table.metadataFileName, 'version-hint.text'],
  )

  // A table that has never been written to reads as no rows, not as a failure.
  assert.equal(table.currentSnapshot, null)
  assert.equal(table.snapshots.length, 0)
  assert.equal(table.manifests().length, 0)
  assert.equal(table.scan().intoTable().numRows, 0)
})

test('an append commits a snapshot, one data file per partition', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const declared = schema()
  // A list of column names is the short spelling of an identity spec.
  const table = iceberg.Table.create(path.join(root, 'trades'), declared, ['venue'])
  table.append(rows([1n, 2n, 3n], ['XNAS', 'XNYS', 'XNAS']))

  const snapshot = table.currentSnapshot
  assert.ok(snapshot instanceof iceberg.Snapshot)
  assert.equal(snapshot.encryptionKeyId, null)
  assert.equal(snapshot.manifests, null)
  assert.equal(snapshot.operation, 'append')
  assert.equal(typeof snapshot.snapshotId, 'bigint')
  assert.equal(snapshot.summary['added-records'], '3')
  const snapshotClone = snapshot.clone()
  assert.notEqual(snapshotClone, snapshot)
  assert.ok(snapshotClone.equals(snapshot))
  assert.equal(snapshotClone.compare(snapshot), 0)
  assert.equal(snapshotClone.stableHash(), snapshot.stableHash())
  assert.equal(typeof snapshot.stableHash(), 'bigint')
  assert.equal(table.snapshots.length, 1)

  const [manifest] = table.manifests()
  assert.ok(manifest instanceof iceberg.ManifestFile)
  assert.equal(manifest.content, 'data')
  assert.equal(manifest.addedFilesCount, 2)
  assert.equal(manifest.addedRowsCount, 3)
  assert.equal(manifest.addedSnapshotId, snapshot.snapshotId)
  assert.ok(Array.isArray(manifest.partitions))
  assert.equal(manifest.keyMetadata, null)
  const manifestClone = manifest.clone()
  assert.notEqual(manifestClone, manifest)
  assert.ok(manifestClone.equals(manifest))
  assert.equal(manifestClone.compare(manifest), 0)
  assert.equal(manifestClone.stableHash(), manifest.stableHash())
  assert.equal(typeof manifest.stableHash(), 'bigint')

  const files = table.dataFiles().sort((left, right) =>
    left.filePath.localeCompare(right.filePath),
  )
  assert.equal(files.length, 2)
  assert.ok(files[0].mimeType instanceof MimeType)
  assert.ok(files[0].mimeType.equals(MimeType.PARQUET))
  assert.deepEqual(files[0].partitionNames, ['venue'])
  assert.equal(files[0].keyMetadata, null)
  assert.equal(files[0].equalityIds, null)
  assert.equal(files[0].firstRowId, null)
  assert.equal(files[0].referencedDataFile, null)
  assert.equal(files[0].contentOffset, null)
  assert.equal(files[0].contentSizeInBytes, null)
  assert.ok(Array.isArray(files[0].nanValueCounts))
  // The manifest is the authority on a partition value, not the directory name.
  assert.deepEqual(
    files.map((file) => file.partition.map((value) => value.asJs())),
    [['XNAS'], ['XNYS']],
  )
  assert.equal(files[0].recordCount + files[1].recordCount, 3)
  assert.ok(files[0].valueCounts.some((entry) => entry.fieldId === 1))
  assert.ok(files[0].toString().includes('venue=XNAS'))
  const clonedFile = files[0].clone()
  assert.notEqual(clonedFile, files[0])
  assert.ok(clonedFile.equals(files[0]))
  assert.equal(clonedFile.compare(files[0]), 0)
  assert.equal(clonedFile.stableHash(), files[0].stableHash())
  assert.notEqual(files[0].compare(files[1]), 0)

  // The Hive layout is a real one: a directory per partition value.
  const data = new IOBase(path.join(root, 'trades', 'data'))
  assert.deepEqual(
    [...data.iterdir()].map((child) => child.name).sort(),
    ['venue=XNAS', 'venue=XNYS'],
  )

  assert.equal(table.scan().intoTable().numRows, 3)
})

test('v3 snapshot lineage stays exact at the JavaScript boundary', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const table = iceberg.Table.create(path.join(root, 'trades'), schema(), undefined, 3)
  table.append(rows([1n, 2n], ['XNAS', 'XNYS']))

  assert.equal(table.formatVersion, 3)
  assert.equal(table.currentSnapshot.encryptionKeyId, null)
  assert.equal(table.currentSnapshot.firstRowId, 0n)
  assert.equal(table.currentSnapshot.addedRows, 2)
})

test('a scan pushes columns down and casts what each file gives back', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const declared = schema()
  const table = iceberg.Table.create(path.join(root, 'trades'), declared)
  table.append(rows([1n, 2n], ['XNAS', 'XNYS']))

  const wanted = fields.struct('row', [declared.dataType.at(0)], { nullable: false })
  const scanned = table.scan(wanted).intoTable()
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
  assert.equal(table.scan().intoTable().numRows, 1)

  // Nothing was mutated in place: the snapshot before it is still recorded.
  assert.equal(table.snapshots.length, 2)
  assert.ok(table.snapshots.some((snapshot) => snapshot.snapshotId === first))
  assert.notEqual(table.snapshots[0].compare(table.snapshots[1]), 0)
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

  const scanned = table.scan().intoTable()
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
  assert.equal(reopened.scan().intoTable().numRows, 2)

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
  assert.equal(spec.fields.length, 1)
  const [partitionField] = spec.fields
  assert.ok(partitionField instanceof iceberg.PartitionField)
  assert.equal(partitionField.sourceId, 2)
  assert.equal(partitionField.fieldId, 1000)
  assert.equal(partitionField.name, 'venue')
  assert.equal(partitionField.transform, 'identity')
  const partitionFieldClone = partitionField.clone()
  assert.notEqual(partitionFieldClone, partitionField)
  assert.ok(partitionFieldClone.equals(partitionField))
  assert.equal(partitionFieldClone.compare(partitionField), 0)
  assert.equal(partitionFieldClone.stableHash(), partitionField.stableHash())
  assert.equal(typeof partitionField.stableHash(), 'bigint')
  const [idField] = iceberg.PartitionSpec.identity(declared, ['id'], 1).fields
  assert.notEqual(partitionField.compare(idField), 0)
  assert.ok(!spec.isUnpartitioned())
  const clonedSpec = spec.clone()
  assert.notEqual(clonedSpec, spec)
  assert.ok(clonedSpec.equals(spec))
  assert.equal(clonedSpec.compare(spec), 0)
  assert.equal(clonedSpec.stableHash(), spec.stableHash())
  const document = {
    'spec-id': 1,
    fields: [
      { name: 'venue', transform: 'identity', 'source-id': 2, 'field-id': 1000 },
    ],
  }
  assert.deepEqual(spec.intoJSON(), document)
  assert.ok(iceberg.PartitionSpec.fromJSON(document).equals(spec))
  const unknown = iceberg.PartitionSpec.fromJSON({
    'spec-id': 2,
    fields: [
      {
        name: 'venue_opaque',
        transform: 'unknown',
        'source-id': 2,
        'field-id': 1001,
      },
      {
        name: 'venue_bucket',
        transform: 'bucket[4294967295]',
        'source-id': 2,
        'field-id': 1002,
      },
    ],
  })
  assert.deepEqual(
    unknown.fields.map((field) => field.transform),
    ['unknown', 'bucket[4294967295]'],
  )
  assert.deepEqual(JSON.parse(JSON.stringify(spec)), document)
  assert.equal(iceberg.PartitionSpec._fromScalarNative, undefined)
  assert.equal(spec._intoScalarNative, undefined)
  const unpartitioned = iceberg.PartitionSpec.unpartitioned()
  assert.ok(unpartitioned.isUnpartitioned())
  assert.notEqual(spec.compare(unpartitioned), 0)

  assert.throws(
    () => iceberg.PartitionSpec.identity(declared, ['nowhere'], 1),
    /nowhere/,
  )
})

test('a catalog maps a dotted name onto folders and creates on first write', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const catalog = new iceberg.Catalog(root)
  assert.equal(catalog.tables.has('nyc.taxis'), false)

  // The first append creates the table from the reader's own schema.
  const table = catalog.append('nyc.taxis', rows([1n, 2n], ['XNAS', 'XNYS']))
  assert.ok(catalog.tables.has('nyc.taxis'))
  assert.equal(table.schema.name, 'row')
  assert.equal(table.schema.dataType.at(0).parquetFieldId, 1)
  assert.equal(table.scan().intoTable().numRows, 2)

  // The dotted name is the folder nyc/taxis, one level per dot.
  const handle = new IOBase(path.join(root, 'nyc', 'taxis'))
  assert.ok(handle.isDir())
  assert.deepEqual(catalog.namespaces.names(), ['nyc'])
  assert.deepEqual(catalog.namespace('nyc').tables.names(), ['taxis'])

  // A second append accumulates rather than replacing.
  const again = catalog.append('nyc.taxis', rows([3n], ['XASE']))
  assert.equal(again.scan().intoTable().numRows, 3)
  assert.equal(catalog.table('nyc.taxis').scan().intoTable().numRows, 3)

  // A schema is a Field, a string expression, or an array of child Fields.
  const rides = catalog.tables.create('nyc.rides', [
    Field.from('id: int64'),
    Field.from('city: utf8'),
  ])
  assert.equal(rides.schema.name, 'row')
  assert.deepEqual(
    Array.from(rides.schema.dataType, (child) => child.name),
    ['id', 'city'],
  )
  catalog.tables.create('nyc.zones', 'row: struct<id int64, zone utf8> not null')
  assert.deepEqual(catalog.namespace('nyc').tables.names(), ['rides', 'taxis', 'zones'])

  // Creating what exists is refused; opening-or-creating is one call.
  assert.throws(() => catalog.tables.create('nyc.taxis', schema()), /nyc\.taxis/)
  const either = catalog.tables.openOrCreate('nyc.taxis', schema())
  assert.equal(either.tableUuid, table.tableUuid)

  // An overwrite through the catalog keeps the previous snapshot readable.
  const replaced = catalog.overwrite('nyc.taxis', rows([9n], ['XNAS']))
  assert.equal(replaced.scan().intoTable().numRows, 1)
  assert.equal(replaced.snapshots.length, 3)
})

test('scanAt reads a retained snapshot after an overwrite', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const table = iceberg.Table.create(path.join(root, 'trades'), schema())
  table.append(rows([1n, 2n], ['XNAS', 'XNYS']))
  const first = table.currentSnapshot.snapshotId
  table.overwrite(rows([3n], ['XASE']))
  assert.equal(table.scan().intoTable().numRows, 1)

  // The overwritten snapshot is still a complete table.
  const past = table.scanAt(first).intoTable()
  assert.equal(past.numRows, 2)
  assert.deepEqual(past.getChild('id').toArray(), BigInt64Array.from([1n, 2n]))

  // Filters are the pairs childrenWhere takes, and a schema keeps the
  // columns it names, exactly as on scan.
  const wanted = fields.struct('row', [Field.from('id: int64')], { nullable: false })
  const filtered = table.scanAt(first, { venue: 'XNAS' }, wanted).intoTable()
  assert.equal(filtered.numCols, 1)
  assert.equal(filtered.numRows, 1)
  assert.deepEqual(filtered.getChild('id').toArray(), BigInt64Array.from([1n]))

  // A snapshot id crosses as a bigint or as an exact number, and an id the
  // table does not retain is refused naming the ids it does.
  assert.throws(() => table.scanAt(7), new RegExp(`got 7; the table retains \\[.*${first}`))
  assert.throws(() => table.scanAt(2 ** 60), /at most 2\^53/)
})

test('a v1 direct-manifest snapshot scans and stays available for time travel', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const location = path.join(root, 'v1')
  const table = iceberg.Table.create(location, schema(), undefined, 1)
  table.append(rows([1n], ['XNAS']))
  const snapshotId = table.currentSnapshot.snapshotId
  const direct = table.manifests().map((manifest) => manifest.manifestPath)

  const metadataPath = path.join(location, 'metadata', table.metadataFileName)
  const encoded = fs.readFileSync(metadataPath, 'utf8')
  const v1 = encoded.replace(
    /"manifest-list"\s*:\s*"[^"]*"/,
    `"manifests":${JSON.stringify(direct)}`,
  )
  assert.notEqual(v1, encoded)
  // Editing the JSON as a JavaScript object would round 64-bit snapshot ids.
  fs.writeFileSync(metadataPath, v1)

  const reopened = iceberg.Table.open(location)
  assert.equal(reopened.currentSnapshot.manifestList, '')
  assert.deepEqual(reopened.currentSnapshot.manifests, direct)
  assert.equal(reopened.manifests().length, 1)
  assert.equal(reopened.scan().intoTable().numRows, 1)

  reopened.append(rows([2n], ['XNYS']))
  assert.equal(reopened.scan().intoTable().numRows, 2)
  assert.equal(reopened.scanAt(snapshotId).intoTable().numRows, 1)
  assert.deepEqual(
    reopened.snapshots.find((snapshot) => snapshot.snapshotId === snapshotId).manifests,
    direct,
  )
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
  const scanned = table.scan().intoTable()
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
  assert.equal(table.inspectFiles().intoTable().numRows, 2)

  const compaction = table.compact()
  assert.ok(compaction instanceof iceberg.Compaction)
  assert.equal(compaction.filesBefore, 2)
  assert.equal(compaction.filesAfter, 1)
  assert.equal(
    compaction.bytesRewritten,
    sizes.reduce((total, size) => total + size, 0),
  )
  assert.equal(table.currentSnapshot.operation, 'replace')
  assert.equal(table.inspectFiles().intoTable().numRows, 1)
  assert.equal(table.scan().intoTable().numRows, 3)
  const compactionClone = compaction.clone()
  assert.notEqual(compactionClone, compaction)
  assert.ok(compactionClone.equals(compaction))
  assert.equal(compactionClone.compare(compaction), 0)
  assert.equal(compactionClone.stableHash(), compaction.stableHash())
  assert.equal(typeof compaction.stableHash(), 'bigint')

  // The pre-compaction snapshot is retained and reads exactly as it did.
  assert.equal(table.scanAt(before).intoTable().numRows, 3)

  // A table with nothing to compact commits nothing and reports zeros.
  const version = table.version
  const noop = table.compact()
  assert.ok(noop instanceof iceberg.Compaction)
  assert.equal(noop.filesBefore, 0)
  assert.equal(noop.filesAfter, 0)
  assert.equal(noop.bytesRewritten, 0)
  assert.notEqual(compaction.compare(noop), 0)
  assert.equal(table.version, version)

  // The inspection tables carry the history under PyIceberg's column names.
  const history = table.inspectHistory().intoTable()
  assert.equal(history.numRows, 3)
  assert.deepEqual(
    history.schema.fields.map((field) => field.name),
    ['made_current_at', 'snapshot_id', 'parent_id', 'is_current_ancestor'],
  )
  const snapshots = table.inspectSnapshots().intoTable()
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
  assert.ok(read.equals(declared, false))
  assert.equal(read.iceberg.get('schema-id'), '0')
  assert.deepEqual(iceberg.schemaToJson(read).asJs(), {
    type: 'struct',
    'schema-id': 0,
    fields: [
      { id: 1, name: 'id', required: false, type: 'long' },
      { id: 2, name: 'venue', required: false, type: 'string' },
    ],
  })

  // A document another catalog handed over reads the same way, as the native
  // value or as the plain object a JSON decoder produced.
  const foreign = {
    type: 'struct',
    'schema-id': 0,
    fields: [{ id: 1, name: 'id', required: true, type: 'long' }],
  }
  assert.ok(
    iceberg
      .schemaFromJson('trade', Scalar.fromJs(foreign))
      .equals(iceberg.schemaFromJson('trade', foreign)),
  )
  const imported = iceberg.schemaFromJson('trade', foreign)
  assert.equal(imported.name, 'trade')
  assert.equal(imported.dataType.length, 1)
  assert.equal(imported.dataType.at(0).nullable, false)
})

test('a namespace is a resource whose collections carry the verbs', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const catalog = new iceberg.Catalog(root)
  const analytics = catalog.namespace('analytics')
  assert.equal(analytics.name, 'analytics')

  // Table access goes through the `tables` collection: open-or-create gets
  // or creates, and doing it again is the same table.
  const schema = new Field('row', 'struct<id: int64, venue: utf8>', false)
  const first = analytics.tables.openOrCreate('trades', schema)
  const same = analytics.tables.openOrCreate('trades', schema)
  assert.equal(same.tableUuid, first.tableUuid)
  assert.ok(analytics.tables.has('trades'))
  assert.deepEqual(analytics.tables.names(), ['trades'])

  // Writing rows through the view replaces the table's rows, creating a
  // table the namespace never had from the rows' own schema.
  analytics.tables.overwrite('quotes', rows([1n, 2n], ['XNAS', 'XNYS']))
  assert.equal(analytics.tables.get('quotes').scan().intoTable().numRows, 2)
  assert.deepEqual(analytics.tables.names().sort(), ['quotes', 'trades'])
  assert.deepEqual(catalog.namespaces.names(), ['analytics'])
})

// The options value is a recording of what a caller set, never a snapshot of
// every field, so each test below asks both halves: what it named, and what it
// left alone.
test('an options value answers the fields it was given and defaults the rest', () => {
  const untouched = new iceberg.IcebergOptions()
  assert.equal(untouched.commitRetries, 4)
  assert.equal(untouched.commitMinBackoffMs, 100)
  assert.equal(untouched.commitMaxBackoffMs, 60_000)
  assert.equal(untouched.commitTotalTimeoutMs, 1_800_000)
  assert.equal(untouched.targetFileSize, 512 * 1024 * 1024)
  assert.equal(untouched.readParallelMinFiles, 16)
  assert.equal(untouched.readParallelMinFileSize, 4 * 1024 * 1024)
  assert.ok(untouched.dataMimeType.equals(MimeType.PARQUET))
  // Nothing compacts on its own until a cadence says so.
  assert.equal(untouched.compactAfterCommits, null)
  // Read parallelism defaults to what the host offers, kept inside 1..=8.
  assert.ok(untouched.readParallelism >= 1 && untouched.readParallelism <= 8)

  const given = new iceberg.IcebergOptions({
    commitRetries: 9,
    commitMinBackoffMs: 5,
    commitMaxBackoffMs: 50,
    commitTotalTimeoutMs: 500,
    targetFileSize: 4096,
    readParallelism: 2,
    readParallelMinFiles: 3,
    readParallelMinFileSize: 1024,
    compactAfterCommits: 7,
    dataMimeType: MimeType.AVRO,
  })
  assert.equal(given.commitRetries, 9)
  assert.equal(given.commitMinBackoffMs, 5)
  assert.equal(given.commitMaxBackoffMs, 50)
  assert.equal(given.commitTotalTimeoutMs, 500)
  assert.equal(given.targetFileSize, 4096)
  assert.equal(given.readParallelism, 2)
  assert.equal(given.readParallelMinFiles, 3)
  assert.equal(given.readParallelMinFileSize, 1024)
  assert.equal(given.compactAfterCommits, 7)
  assert.ok(given.dataMimeType.equals(MimeType.AVRO))
  const cloned = given.clone()
  assert.notEqual(cloned, given)
  assert.ok(cloned.equals(given))
  assert.equal(cloned.compare(given), 0)
  assert.equal(cloned.stableHash(), given.stableHash())
  cloned.commitRetries = 10
  assert.ok(!cloned.equals(given))
  assert.notEqual(cloned.compare(given), 0)
  assert.equal(given.commitRetries, 9)

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

  for (const invalid of [-1, 1.5, 2 ** 54]) {
    assert.throws(
      () => new iceberg.IcebergOptions({ commitTotalTimeoutMs: invalid }),
      /commitTotalTimeoutMs/,
    )
  }
})

test('property-derived option integers never round at the JavaScript boundary', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const table = iceberg.Table.create(root, fields.struct('row', [Field.from('id: int64')], { nullable: false }))
  table.updateProperties({ 'commit.retry.total-timeout-ms': String(2 ** 54) })
  const options = table.options()
  assert.throws(() => options.commitTotalTimeoutMs, /cannot be represented exactly/)
})

test('a data MIME type accepts native/parser input and rejects unsupported types', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const options = new iceberg.IcebergOptions({ dataMimeType: MimeType.AVRO })
  assert.ok(options.dataMimeType.equals(MimeType.AVRO))
  options.dataMimeType = 'PaRqUeT'
  assert.ok(options.dataMimeType.equals(MimeType.PARQUET))
  assert.ok(
    new iceberg.IcebergOptions({ dataMimeType: 'application/avro' })
      .dataMimeType.equals(MimeType.AVRO),
  )
  const puffin = new iceberg.IcebergOptions({ dataMimeType: MimeType.PUFFIN })
  assert.ok(puffin.dataMimeType.equals(MimeType.PUFFIN))
  const table = iceberg.Table.create(path.join(root, 'trades'), schema())
  assert.throws(
    () => table.append(rows([1n], ['XNAS']), puffin),
    /write\.format\.default.*puffin/i,
  )
  assert.equal(table.currentSnapshot, null)

  assert.throws(
    () => new iceberg.IcebergOptions({ dataMimeType: MimeType.CSV }),
    /data MIME type.*parquet.*avro.*orc.*puffin/i,
  )
  assert.throws(() => {
    options.dataMimeType = 'csv'
  }, /data MIME type.*puffin/i)
  // A refused write leaves the field holding what it held.
  assert.ok(options.dataMimeType.equals(MimeType.PARQUET))
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

test('a per-call data MIME type writes AVRO files beside the PARQUET ones', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const table = iceberg.Table.create(path.join(root, 'trades'), schema())
  table.append(rows([1n, 2n], ['XNAS', 'XNYS']))
  table.append(rows([3n], ['XASE']), new iceberg.IcebergOptions({ dataMimeType: 'avro' }))

  // The format is the assertion. A wrapper that dropped the options argument
  // would still commit, still return, and still scan - and write Parquet
  // twice, which nothing but the manifest entry would reveal.
  assert.deepEqual(
    dataMimeTypes(table),
    [MimeType.AVRO.toString(), MimeType.PARQUET.toString()],
  )

  // A scan decodes each file as the format its manifest entry records, so a
  // table of two formats still reads as one shape.
  const scanned = table.scan().intoTable()
  assert.equal(scanned.numRows, 3)
  assert.deepEqual(
    [...scanned.getChild('id')].sort((left, right) => Number(left - right)),
    [1n, 2n, 3n],
  )
  // The call configured itself alone: nothing was stored on the handle.
  assert.ok(table.options().dataMimeType.equals(MimeType.PARQUET))
})

test('setOptions is what later calls resolve, and a per-call option outlives only its call', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const table = iceberg.Table.create(path.join(root, 'trades'), schema())
  table.setOptions(new iceberg.IcebergOptions({ targetFileSize: 4096, dataMimeType: MimeType.AVRO }))
  assert.ok(table.options().dataMimeType.equals(MimeType.AVRO))
  // The override shadows the table property the getter would otherwise read.
  assert.equal(table.targetFileSize, 4096)

  table.append(rows([1n], ['XNAS']))
  assert.deepEqual(dataMimeTypes(table), [MimeType.AVRO.toString()])

  table.append(rows([2n], ['XNYS']), new iceberg.IcebergOptions({ dataMimeType: MimeType.PARQUET }))
  assert.deepEqual(
    dataMimeTypes(table),
    [MimeType.AVRO.toString(), MimeType.PARQUET.toString()],
  )

  // The override is put back whatever the call did, and a field the per-call
  // value never named was never in question.
  assert.ok(table.options().dataMimeType.equals(MimeType.AVRO))
  assert.equal(table.options().targetFileSize, 4096)
})

// Testing the per-call option on `append` alone is what let three methods ship
// accepting an options argument they never declared: the JS wrapper forwarded
// it, napi discarded the extra, and every one of them committed, returned, and
// scanned exactly as if it had worked. One case per write is the only shape of
// this test that would have caught it.
test('every write that takes a per-call data MIME type actually writes it', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const avro = () => new iceberg.IcebergOptions({ dataMimeType: MimeType.AVRO })

  const overwritten = iceberg.Table.create(path.join(root, 'ow'), schema(), ['venue'])
  overwritten.append(rows([1n, 2n], ['XNAS', 'XNYS']))
  overwritten.overwriteWhere({ venue: 'XNAS' }, rows([9n], ['XNAS']), avro())
  // XNYS is carried forward as the PARQUET file it already was; only the
  // partition this call rewrote is AVRO.
  assert.deepEqual(dataMimeTypes(overwritten), [MimeType.AVRO.toString(), MimeType.PARQUET.toString()])

  const merged = iceberg.Table.create(path.join(root, 'mg'), schema())
  merged.append(rows([1n, 2n], ['XNAS', 'XNYS']))
  merged.merge(rows([2n, 3n], ['XLON', 'XASE']), ['id'], true, avro())
  assert.deepEqual(dataMimeTypes(merged), [MimeType.AVRO.toString()])
  assert.equal(merged.scan().intoTable().numRows, 3)

  const mergedWhere = iceberg.Table.create(path.join(root, 'mw'), schema(), ['venue'])
  mergedWhere.append(rows([1n, 2n], ['XNAS', 'XNYS']))
  mergedWhere.mergeWhere({ venue: 'XNAS' }, rows([1n], ['XNAS']), ['id'], true, avro())
  assert.deepEqual(dataMimeTypes(mergedWhere), [MimeType.AVRO.toString(), MimeType.PARQUET.toString()])

  const replaced = iceberg.Table.create(path.join(root, 'ov'), schema())
  replaced.append(rows([1n], ['XNAS']))
  replaced.overwrite(rows([2n], ['XNYS']), avro())
  assert.deepEqual(dataMimeTypes(replaced), [MimeType.AVRO.toString()])

  // None of them stored anything on the handle.
  for (const table of [overwritten, merged, mergedWhere, replaced]) {
    assert.ok(table.options().dataMimeType.equals(MimeType.PARQUET))
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
  catalog.tables.create('sales.orders', schema())
  catalog.append('sales.orders', rows([1n], ['XNAS']), new iceberg.IcebergOptions({ dataMimeType: MimeType.AVRO }))
  assert.deepEqual(dataMimeTypes(catalog.table('sales.orders')), [MimeType.AVRO.toString()])

  catalog.overwrite('sales.orders', rows([2n], ['XNYS']), new iceberg.IcebergOptions({ dataMimeType: MimeType.AVRO }))
  assert.deepEqual(dataMimeTypes(catalog.table('sales.orders')), [MimeType.AVRO.toString()])

  // The view spelling agrees, which is the whole point of there being two.
  catalog.namespaces.get('sales').tables.append('orders', rows([3n], ['XLON']))
  assert.deepEqual(
    dataMimeTypes(catalog.table('sales.orders')),
    [MimeType.AVRO.toString(), MimeType.PARQUET.toString()],
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
  assert.equal(table.scanWhere({ venue: 'XNAS' }, null, options).intoTable().numRows, 1)
  assert.equal(table.scanRef('release', null, null, options).intoTable().numRows, 2)
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
  assert.equal(table.scanRef('audit').intoTable().numRows, 2)
  assert.equal(table.scanRef('audit', { venue: 'XNAS' }).intoTable().numRows, 1)

  const removed = table.removeRef('nightly')
  assert.ok(removed instanceof iceberg.SnapshotRef)
  assert.equal(removed.snapshotId, first)
  assert.equal(removed.kind, 'tag')
  const removedClone = removed.clone()
  assert.notEqual(removedClone, removed)
  assert.ok(removedClone.equals(removed))
  assert.equal(removedClone.compare(removed), 0)
  assert.equal(removedClone.stableHash(), removed.stableHash())
  assert.equal(typeof removed.stableHash(), 'bigint')

  // Dropping a ref that was never there is a typo far more often than it is a
  // no-op, so it is refused naming the refs the table does have.
  assert.throws(() => table.removeRef('nightly'), /got "nightly"; it has \[audit, main\]/)
  assert.equal(table.snapshotByRef('audit').snapshotId, first)
  const branch = table.removeRef('audit')
  assert.notEqual(removed.compare(branch), 0)
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
  assert.equal(table.scanRef('audit').intoTable().numRows, 1)
  table.fastForward('audit', second)
  assert.equal(table.snapshotByRef('audit').snapshotId, second)
  assert.equal(table.scanRef('audit').intoTable().numRows, 2)

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

  const copy = everything.clone()
  assert.notEqual(copy, everything)
  assert.ok(copy.equals(everything))
  assert.equal(copy.compare(everything), 0)
  assert.equal(copy.stableHash(), everything.stableHash())
  assert.equal(typeof everything.stableHash(), 'bigint')

  // Pruning is a number rather than a promise: one manifest was ruled out
  // whole by the manifest list's summaries, one file inside the other by its
  // partition tuple, and the rows left agree with what a scan yields.
  const matching = table.plan({ venue: 'XNAS' })
  assert.equal(matching.filesPlanned, 1)
  assert.equal(matching.filesSkipped, 1)
  assert.equal(matching.manifestsRead, 1)
  assert.equal(matching.manifestsSkipped, 1)
  assert.equal(matching.recordCount, table.scanWhere({ venue: 'XNAS' }).intoTable().numRows)
  assert.ok(!everything.equals(matching))
  assert.ok(everything.compare(matching) > 0)

  // ScanPlan is deliberately the bounded public report, not an exposed task
  // list. An equivalent plan over different physical paths therefore has the
  // same value identity even though its hidden scan tasks cannot be equal.
  const mirror = iceberg.Table.create(path.join(root, 'mirror'), schema(), ['venue'])
  mirror.append(rows([1n, 2n, 3n], ['XNAS', 'XNYS', 'XNAS']))
  mirror.append(rows([4n], ['XASE']))
  assert.notEqual(table.dataFiles()[0].filePath, mirror.dataFiles()[0].filePath)
  const mirrorReport = mirror.plan()
  assert.ok(everything.equals(mirrorReport))
  assert.equal(everything.compare(mirrorReport), 0)
  assert.equal(everything.stableHash(), mirrorReport.stableHash())

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

  const current = table.manifests()
  assert.equal(current.length, 2)
  const past = table.manifestsAt(first)
  assert.equal(past.length, 1)
  assert.equal(past[0].addedSnapshotId, first)
  assert.equal(past[0].addedRowsCount, 1)
  assert.notEqual(current[1].compare(past[0]), 0)

  // An id the table does not retain is refused naming the ids it does.
  assert.throws(() => table.manifestsAt(7), new RegExp(`got 7; the table retains \\[.*${first}`))
})

test('scanWhere reads only the partition it names and projects as scan does', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const table = iceberg.Table.create(path.join(root, 'trades'), schema(), ['venue'])
  table.append(rows([1n, 2n, 3n], ['XNAS', 'XNYS', 'XNAS']))

  const matched = table.scanWhere({ venue: 'XNAS' }).intoTable()
  assert.equal(matched.numRows, 2)
  assert.deepEqual([...matched.getChild('venue')], ['XNAS', 'XNAS'])

  // The projection sits after the filters, and keeps the columns it names.
  const projected = table.scanWhere({ venue: 'XNAS' }, 'row: struct<id int64> not null').intoTable()
  assert.equal(projected.numCols, 1)
  assert.deepEqual([...projected.getChild('id')], [1n, 3n])

  // Filters also spell as the `{column, value}` entries a manifest reports.
  assert.equal(
    table.scanWhere([{ column: 'venue', value: 'XNYS' }]).intoTable().numRows,
    1,
  )

  // A value nothing was ever written under is no rows, with the shape intact.
  const none = table.scanWhere({ venue: 'XLON' }).intoTable()
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
    [...table.scan().intoTable().getChild('id')].sort((left, right) => Number(left - right)),
    [2n, 9n],
  )
  // Only the current pointer moved: the snapshot before it is still whole.
  assert.equal(table.scanAt(first).intoTable().numRows, 3)
})

test('merge updates the rows whose key is stored and appends the rest', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const table = iceberg.Table.create(path.join(root, 'trades'), schema())
  table.append(rows([1n, 2n], ['XNAS', 'XNYS']))
  table.merge(rows([2n, 3n], ['XASE', 'XLON']), ['id'])

  const scanned = table.scan().intoTable()
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
  assert.equal(table.scan().intoTable().numRows, 1)
  assert.deepEqual([...table.scan().intoTable().getChild('venue')], ['XPAR'])
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
  assert.equal(table.scanWhere({ venue: 'XNAS' }).intoTable().numRows, 3)
  assert.equal(table.scan().intoTable().numRows, 4)
})

test('expireSnapshots supports defaults, retain overrides, and explicit ids', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const table = iceberg.Table.create(path.join(root, 'trades'), schema())
  table.append(rows([1n], ['XNAS']))
  const first = table.currentSnapshot.snapshotId
  table.append(rows([2n], ['XNYS']))
  assert.equal(table.snapshots.length, 2)

  // Fresh snapshots survive the default cutoff. Retaining two also protects
  // both from a future cutoff, and an unknown explicit id is ignored.
  const version = table.version
  assert.deepEqual(table.expireSnapshots(), [])
  assert.deepEqual(table.expireSnapshots(Date.now() + 60_000, 2), [])
  assert.deepEqual(table.expireSnapshots(1, undefined, [999n]), [])
  assert.deepEqual(table.expireSnapshots(1), [])
  assert.equal(table.version, version)
  assert.equal(table.snapshots.length, 2)

  assert.throws(() => table.expireSnapshots(undefined, 0), /retain_last.*at least 1/)
  assert.throws(
    () => table.expireSnapshots(undefined, undefined, [table.currentSnapshot.snapshotId]),
    /cannot expire current snapshot/,
  )

  // Explicit ids join age selection, so an old cutoff does not protect this
  // known, unreferenced ancestor.
  assert.deepEqual(table.expireSnapshots(1, undefined, [first]), [first])
  assert.equal(table.snapshots.length, 1)
  assert.ok(table.version > version)
  // Expiry drops history, never rows: what the current snapshot holds is what
  // it held before the ancestors went away.
  assert.equal(table.scan().intoTable().numRows, 2)
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

  assert.throws(() => namespaces.get('sales'), /expected a namespace at "sales", got nothing/)

  // Both views were built before the write and answer storage at call time, so
  // both see it without being rebuilt.
  catalog.append('sales.orders', rows([1n], ['XNAS']))
  assert.deepEqual(namespaces.names(), ['sales'])
  assert.equal(namespaces.size(), 1)
  assert.deepEqual(orders.names(), ['orders'])
  assert.equal(orders.size(), 1)
  assert.equal(orders.has('orders'), true)
  assert.throws(() => orders.get('ledger'), /expected a table at "sales\.ledger", got nothing/)

  // One spelling chains from the catalog to the rows.
  assert.equal(catalog.namespaces.get('sales').tables.get('orders').scan().intoTable().numRows, 1)
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
    /expected to create a table at "sales\.quotes", got an existing table/,
  )

  // A write through the view creates the table from the rows' own schema, and
  // forwards the trailing options the way the table's own writes do.
  tables.append('orders', rows([1n], ['XNAS']), new iceberg.IcebergOptions({ dataMimeType: MimeType.AVRO }))
  tables.append('orders', rows([2n], ['XNYS']))
  assert.deepEqual(
    dataMimeTypes(tables.get('orders')),
    [MimeType.AVRO.toString(), MimeType.PARQUET.toString()],
  )

  const replaced = tables.overwrite(
    'orders',
    rows([3n], ['XASE']),
    new iceberg.IcebergOptions({ dataMimeType: MimeType.AVRO }),
  )
  assert.deepEqual(dataMimeTypes(replaced), [MimeType.AVRO.toString()])
  assert.equal(replaced.scan().intoTable().numRows, 1)
  assert.deepEqual(tables.names(), ['orders', 'quotes'])
  assert.equal(tables.size(), 2)
})

test('a dotted create into an empty warehouse is one call', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const catalog = new iceberg.Catalog(root)

  // The namespace view exists before its folder does, so the chain writes
  // into an empty warehouse: the table's first metadata document is what
  // brings every ancestor namespace into being.
  const created = catalog.namespace('sales.eu').tables.create('orders', schema())

  // The same table, every spelling: the catalog's dotted entry point, the
  // root tables view, and the chained Map lookups.
  assert.equal(catalog.table('sales.eu.orders').tableUuid, created.tableUuid)
  assert.equal(catalog.tables.get('sales.eu.orders').tableUuid, created.tableUuid)
  assert.ok(catalog.tables.has('sales.eu.orders'))
  const chained = catalog.namespaces.get('sales.eu').tables.get('orders')
  assert.equal(chained.tableUuid, created.tableUuid)

  // The root tables view lists tables directly under the warehouse, so a
  // table two namespaces down is reached by name, not by listing.
  assert.deepEqual(catalog.tables.names(), [])
})

test('the views speak the whole Map vocabulary, lazily', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const catalog = new iceberg.Catalog(root)
  const sales = catalog.namespaces.create('sales')
  sales.tables.create('orders', schema())
  sales.tables.create('returns', schema())
  sales.namespaces.create('eu')

  // has, size, keys, values, entries, and for...of - the Map verbs are the
  // spelling, because JavaScript has no indexing hook a native class can
  // answer, and the docs say so instead of emulating operator sugar.
  assert.ok(catalog.namespaces.has('sales'))
  assert.equal(catalog.namespaces.size(), 1)
  assert.deepEqual([...catalog.namespaces.keys()], ['sales'])
  assert.deepEqual([...catalog.namespaces.values()].map((view) => view.name), ['sales'])
  assert.deepEqual(
    [...catalog.namespaces.entries()].map(([name, view]) => [name, view.name]),
    [['sales', 'sales']],
  )
  const walked = []
  for (const name of catalog.namespaces) walked.push(name)
  assert.deepEqual(walked, ['sales'])

  assert.ok(sales.tables.has('orders'))
  assert.equal(sales.tables.size(), 2)
  assert.deepEqual([...sales.tables.keys()], ['orders', 'returns'])
  assert.deepEqual(
    [...sales.tables.entries()].map(([name, table]) => [name, table.location]),
    [
      ['orders', sales.tables.get('orders').location],
      ['returns', sales.tables.get('returns').location],
    ],
  )
  for (const name of sales.tables) walked.push(name)
  assert.deepEqual(walked, ['sales', 'orders', 'returns'])

  // values() opens one table per step: a sibling whose metadata document is
  // broken poisons the drain, never the first value.
  const poisoned = path.join(root, 'sales', 'zzz', 'metadata')
  fs.mkdirSync(poisoned, { recursive: true })
  fs.writeFileSync(path.join(poisoned, 'v1.metadata.json'), '{}')
  const values = sales.tables.values()
  assert.equal(values.next().value.root.name, 'orders')
  assert.throws(() => [...values])
})

test('a catalog and a namespace carry properties, transactionally', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const catalog = new iceberg.Catalog(path.join(root, 'warehouse'))

  // Absent means empty, and a call given nothing writes nothing.
  assert.deepEqual(catalog.properties(), {})
  catalog.updateProperties()
  assert.ok(!fs.existsSync(path.join(root, 'warehouse')))

  catalog.updateProperties({ owner: 'finance' })
  assert.deepEqual(catalog.properties(), { owner: 'finance' })
  catalog.updateProperties(new Map([['region', 'eu']]), ['owner'])
  assert.deepEqual(catalog.properties(), { region: 'eu' })

  // The reserved prefix is refused with the core's own message.
  assert.throws(() => catalog.updateProperties({ 'iceberg:x': '1' }), /reserved "iceberg:"/)

  const sales = catalog.namespaces.create('sales')
  assert.deepEqual(sales.properties(), {})
  sales.updateProperties({ team: 'emea' })
  assert.deepEqual(sales.properties(), { team: 'emea' })
  assert.deepEqual(catalog.namespaces.get('sales').properties(), { team: 'emea' })
  assert.throws(() => sales.updateProperties({ 'iceberg:x': '1' }), /reserved "iceberg:"/)
})
