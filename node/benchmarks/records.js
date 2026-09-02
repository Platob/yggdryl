'use strict'

const fs = require('node:fs')
const os = require('node:os')
const { performance } = require('node:perf_hooks')
const path = require('node:path')
const arrow = require('apache-arrow')
const {
  BatchReader,
  Field,
  IOBase,
  MimeType,
  RecordOptions,
  fields,
  iceberg,
} = require('yggdryl')

// A record crossing is far more work than a schema lookup, so this target
// counts in thousands rather than tens of thousands by default. It also keeps
// Parquet footer and projected-WKB Scalar crossings beside the record surface.
// Smoke mode keeps fixtures and loops small enough for package verification.
const smoke = process.env.YGGDRYL_BENCH_SMOKE === '1'
const iterations = Number.parseInt(
  smoke ? '1' : (process.env.YGGDRYL_BENCH_ITERATIONS ?? '200'),
  10,
)
if (!Number.isSafeInteger(iterations) || iterations <= 0) {
  throw new RangeError('YGGDRYL_BENCH_ITERATIONS must be a positive safe integer')
}
const warmups = smoke ? 0 : Math.min(iterations, 20)
const filter = process.env.YGGDRYL_BENCH_FILTER

function selected(name) {
  return filter === undefined || name.includes(filter)
}

function benchmark(name, operation) {
  if (!selected(name)) return
  for (let index = 0; index < warmups; index += 1) operation()
  const started = performance.now()
  for (let index = 0; index < iterations; index += 1) operation()
  const elapsed = performance.now() - started
  const rate = Math.round((iterations * 1_000) / elapsed)
  console.log(`${name}: ${rate.toLocaleString('en-US')} operations/second`)
}

const rows = smoke ? 64 : 10_000
const commitRowSize = smoke ? 16 : 1_000
const writeBatchSize = smoke ? 32 : 1_024
// A plain array of bigints, not a `BigInt64Array`: Apache Arrow JS reads a
// typed array as a column that cannot be null, and the declared root below says
// every column can be.
const ids = new Array(rows)
const symbols = new Array(rows)
const venues = new Array(rows)
for (let index = 0; index < rows; index += 1) {
  ids[index] = BigInt(index)
  symbols[index] = index % 2 === 0 ? 'AAPL' : 'MSFT'
  venues[index] = index % 3 === 0 ? 'XNAS' : 'XNYS'
}

// Every fixture is built once, outside the measured loops: the numbers report
// the boundary crossing, not Apache Arrow JS building a column.
const table = new arrow.Table({
  id: arrow.vectorFromArray(ids, new arrow.Int64()),
  symbol: arrow.vectorFromArray(symbols, new arrow.Utf8()),
  venue: arrow.vectorFromArray(venues, new arrow.Utf8()),
})
const ipc = arrow.tableToIPC(table)
const schema = fields.struct(
  'row',
  [Field.from('id: int64'), Field.from('symbol: utf8'), Field.from('venue: utf8')],
  { nullable: false },
)
const wanted = fields.struct('row', [Field.from('id: int64')], { nullable: false })

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-bench-'))
const stream = new IOBase(path.join(root, 'trades.arrows'))
stream.overwriteArrowReader(BatchReader.from(ipc))
const parquet = new IOBase(path.join(root, 'trades.parquet'))
parquet.overwriteArrowTable(table, parquet.recordOptions().withField(schema))
const wkbPoint = Buffer.allocUnsafe(21)
wkbPoint.writeUInt8(1, 0)
wkbPoint.writeUInt32LE(1, 1)
wkbPoint.writeDoubleLE(1, 5)
wkbPoint.writeDoubleLE(2, 13)
const geospatial = new IOBase(path.join(root, 'shapes.parquet'))
geospatial.overwriteArrowTable(
  new arrow.Table({
    shape: arrow.vectorFromArray(new Array(rows).fill(wkbPoint), new arrow.Binary()),
  }),
)

const memory = IOBase.fromBytes(stream.readBytes())
memory.mediaType = MimeType.ARROW_STREAM

benchmark('records/reader_from_ipc', () => BatchReader.fromIpc(ipc))
benchmark('records/reader_from_arrow_table', () => BatchReader.from(table))
benchmark('records/record_options', () => memory.recordOptions())
const protocolOptions = memory.recordOptions().withField(schema).withBatchSize(writeBatchSize)
benchmark('records/record_options_equals', () => protocolOptions.equals(protocolOptions))
benchmark('records/record_options_compare', () => protocolOptions.compare(protocolOptions))
benchmark('records/record_options_stable_hash', () => protocolOptions.stableHash())
benchmark('records/record_options_clone', () => protocolOptions.clone())
const avroOptions = RecordOptions.from('benchmark.avro')
const avroSyncMarker = Buffer.from('0123456789abcdef')
benchmark('records/avro_option_values', () => [
  avroOptions.blockCodec,
  avroOptions.syncMarker,
])
benchmark('records/avro_option_setters', () => {
  avroOptions.blockCodec = 'deflate'
  avroOptions.syncMarker = avroSyncMarker
})
benchmark('records/read_field_ipc', () => memory.readArrowField())
benchmark('records/read_parquet_statistics', () => parquet.readParquetStatistics())
benchmark('records/read_parquet_geospatial_statistics', () =>
  geospatial.readParquetGeospatialStatistics('shape'),
)
const memoryWanted = memory.recordOptions().withField(wanted)
const parquetWanted = parquet.recordOptions().withField(wanted)
benchmark('records/read_ipc_into_ipc', () => memory.readArrowReader().intoIpc())
benchmark('records/read_ipc_into_arrow', () => memory.readArrowReader().intoTable())
benchmark('records/read_ipc_pushdown', () => memory.readArrowReader(memoryWanted).intoIpc())
benchmark('records/read_parquet_into_ipc', () => parquet.readArrowReader().intoIpc())
benchmark('records/read_parquet_pushdown', () => parquet.readArrowReader(parquetWanted).intoIpc())

// Each explicit representation adapter is benchmarked under every intent. A
// fresh in-memory handle keeps append/merge iterations bounded instead of
// growing one benchmark fixture forever.
const records = new Array(rows)
for (let index = 0; index < rows; index += 1) {
  records[index] = { id: ids[index], symbol: symbols[index], venue: venues[index] }
}
const storedBytes = stream.readBytes()
function stored() {
  const handle = IOBase.fromBytes(storedBytes)
  handle.mediaType = MimeType.ARROW_STREAM
  return handle
}
function keyed(handle) {
  return handle.recordOptions().withMergeByNames(['id'])
}
function committed(handle, intent) {
  let options = handle
    .recordOptions()
    .withField(schema)
    .withBatchSize(writeBatchSize)
    .withCommitRowSize(commitRowSize)
  if (intent === 'merge') options = options.withMergeByNames(['id'])
  return options
}

benchmark('records/overwrite_arrow_reader', () =>
  stored().overwriteArrowReader(BatchReader.fromIpc(ipc)),
)
benchmark('records/append_arrow_reader', () =>
  stored().appendArrowReader(BatchReader.fromIpc(ipc)),
)
benchmark('records/merge_arrow_reader', () => {
  const handle = stored()
  handle.mergeArrowReader(BatchReader.fromIpc(ipc), keyed(handle))
})
benchmark('records/overwrite_arrow_reader_commit_rows', () => {
  const handle = stored()
  handle.overwriteArrowReader(BatchReader.fromIpc(ipc), committed(handle, 'overwrite'))
})
benchmark('records/append_arrow_reader_commit_rows', () => {
  const handle = stored()
  handle.appendArrowReader(BatchReader.fromIpc(ipc), committed(handle, 'append'))
})
benchmark('records/merge_arrow_reader_commit_rows', () => {
  const handle = stored()
  handle.mergeArrowReader(BatchReader.fromIpc(ipc), committed(handle, 'merge'))
})

benchmark('records/overwrite_arrow_table', () => stored().overwriteArrowTable(table))
benchmark('records/append_arrow_table', () => stored().appendArrowTable(table))
benchmark('records/merge_arrow_table', () => {
  const handle = stored()
  handle.mergeArrowTable(table, keyed(handle))
})

benchmark('records/overwrite_arrow_batch', () =>
  stored().overwriteArrowBatch(table.batches[0]),
)
benchmark('records/append_arrow_batch', () =>
  stored().appendArrowBatch(table.batches[0]),
)
benchmark('records/merge_arrow_batch', () => {
  const handle = stored()
  handle.mergeArrowBatch(table.batches[0], keyed(handle))
})

benchmark('records/overwrite_records', () => stored().overwriteRecords(records))
function* recordGenerator() {
  yield* records
}
benchmark('records/overwrite_record_generator', () =>
  stored().overwriteRecords(recordGenerator()),
)
benchmark('records/overwrite_record_generator_commit_rows', () => {
  const handle = stored()
  handle.overwriteRecords(recordGenerator(), committed(handle, 'overwrite'))
})
benchmark('records/append_record_generator_commit_rows', () => {
  const handle = stored()
  handle.appendRecords(recordGenerator(), committed(handle, 'append'))
})
benchmark('records/merge_record_generator_commit_rows', () => {
  const handle = stored()
  handle.mergeRecords(recordGenerator(), committed(handle, 'merge'))
})
benchmark('records/append_records', () => stored().appendRecords(records))
benchmark('records/merge_records', () => {
  const handle = stored()
  handle.mergeRecords(records, keyed(handle))
})

// The configurable surface adds only mode dispatch; keep each public input
// shape visible so benchmark regressions cannot hide in a binding adapter.
for (const mode of ['overwrite', 'append', 'merge']) {
  benchmark(`records/write_arrow_reader/${mode}`, () => {
    const handle = stored()
    handle.writeArrowReader(
      BatchReader.fromIpc(ipc),
      mode,
      mode === 'merge' ? keyed(handle) : undefined,
    )
  })
  benchmark(`records/write_arrow_table/${mode}`, () => {
    const handle = stored()
    handle.writeArrowTable(
      table,
      mode,
      mode === 'merge' ? keyed(handle) : undefined,
    )
  })
  benchmark(`records/write_arrow_batch/${mode}`, () => {
    const handle = stored()
    handle.writeArrowBatch(
      table.batches[0],
      mode,
      mode === 'merge' ? keyed(handle) : undefined,
    )
  })
  benchmark(`records/write_records/${mode}`, () => {
    const handle = stored()
    handle.writeRecords(
      records,
      mode,
      mode === 'merge' ? keyed(handle) : undefined,
    )
  })
}

// One commit writes a data file, a manifest, a manifest list, and a metadata
// document, so an Iceberg append is measured separately from a plain write.
const iced = iceberg.assignFieldIds(schema)
const lake = path.join(root, 'lake')
const iceTable = iceberg.Table.create(lake, iced)
iceTable.append(ipc)
const iceFile = iceTable.dataFiles()[0]
const icePlan = iceTable.plan()
const icePartitionField = iceberg.PartitionSpec.identity(iced, ['venue']).fields[0]
const iceSnapshot = iceTable.currentSnapshot
const iceManifest = iceTable.manifests()[0]
iceTable.createTag('benchmark-value-protocol', iceSnapshot.snapshotId)
const iceSnapshotRef = iceTable.removeRef('benchmark-value-protocol')
// One live file has no compaction group, so this bounded report is produced
// without another snapshot or data rewrite.
const iceCompaction = iceTable.compact()
const iceOptions = new iceberg.IcebergOptions({
  commitRetries: 4,
  targetFileSize: 512 * 1024 * 1024,
  readParallelism: 4,
  dataMimeType: MimeType.AVRO,
})

// What the widened write costs at the boundary. Each sample commits into its
// own fresh table, because a growing manifest list would otherwise time the
// table's history rather than the write - the same rule the Python Iceberg
// benchmark follows.
const iceRows = Array.from({ length: 64 }, (_, index) => ({
  id: BigInt(index),
  venue: 'XNAS',
  price: index / 4,
}))
let iceBenchIndex = 0
const freshIceTable = () =>
  iceberg.Table.create(path.join(root, 'bench', `t${iceBenchIndex++}`), iced)
benchmark('iceberg/append_arrow_ipc', () => freshIceTable().append(ipc))
benchmark('iceberg/append_rows', () => freshIceTable().append(iceRows))

benchmark('iceberg/scan_into_ipc', () => iceTable.scan().intoIpc())
benchmark('iceberg/scan_pushdown', () =>
  iceTable.scan(fields.struct('row', [iced.dataType.at(0)], { nullable: false })).intoIpc(),
)
benchmark('iceberg/data_files', () => iceTable.dataFiles())
benchmark('iceberg/data_file_equals', () => iceFile.equals(iceFile))
benchmark('iceberg/data_file_compare', () => iceFile.compare(iceFile))
benchmark('iceberg/data_file_stable_hash', () => iceFile.stableHash())
benchmark('iceberg/data_file_clone', () => iceFile.clone())
benchmark('iceberg/scan_plan_equals', () => icePlan.equals(icePlan))
benchmark('iceberg/scan_plan_compare', () => icePlan.compare(icePlan))
benchmark('iceberg/scan_plan_stable_hash', () => icePlan.stableHash())
benchmark('iceberg/scan_plan_clone', () => icePlan.clone())
benchmark('iceberg/partition_field_equals', () => icePartitionField.equals(icePartitionField))
benchmark('iceberg/partition_field_compare', () => icePartitionField.compare(icePartitionField))
benchmark('iceberg/partition_field_stable_hash', () => icePartitionField.stableHash())
benchmark('iceberg/partition_field_clone', () => icePartitionField.clone())
benchmark('iceberg/snapshot_equals', () => iceSnapshot.equals(iceSnapshot))
benchmark('iceberg/snapshot_compare', () => iceSnapshot.compare(iceSnapshot))
benchmark('iceberg/snapshot_stable_hash', () => iceSnapshot.stableHash())
benchmark('iceberg/snapshot_clone', () => iceSnapshot.clone())
benchmark('iceberg/snapshot_ref_equals', () => iceSnapshotRef.equals(iceSnapshotRef))
benchmark('iceberg/snapshot_ref_compare', () => iceSnapshotRef.compare(iceSnapshotRef))
benchmark('iceberg/snapshot_ref_stable_hash', () => iceSnapshotRef.stableHash())
benchmark('iceberg/snapshot_ref_clone', () => iceSnapshotRef.clone())
benchmark('iceberg/manifest_file_equals', () => iceManifest.equals(iceManifest))
benchmark('iceberg/manifest_file_compare', () => iceManifest.compare(iceManifest))
benchmark('iceberg/manifest_file_stable_hash', () => iceManifest.stableHash())
benchmark('iceberg/manifest_file_clone', () => iceManifest.clone())
benchmark('iceberg/compaction_equals', () => iceCompaction.equals(iceCompaction))
benchmark('iceberg/compaction_compare', () => iceCompaction.compare(iceCompaction))
benchmark('iceberg/compaction_stable_hash', () => iceCompaction.stableHash())
benchmark('iceberg/compaction_clone', () => iceCompaction.clone())
benchmark('iceberg/options_equals', () => iceOptions.equals(iceOptions))
benchmark('iceberg/options_compare', () => iceOptions.compare(iceOptions))
benchmark('iceberg/options_stable_hash', () => iceOptions.stableHash())
benchmark('iceberg/options_clone', () => iceOptions.clone())
benchmark('iceberg/open', () => iceberg.Table.open(lake))

// A catalog append crosses the whole boundary a caller with only rows and a
// name uses: resolve the dotted name against the warehouse, locate the table
// there, and commit one snapshot. The rows stay small so the number reports
// that path rather than Parquet encoding.
const catalog = new iceberg.Catalog(path.join(root, 'warehouse'))
const catalogRows = arrow.tableToIPC(
  new arrow.Table({
    id: arrow.vectorFromArray([1n, 2n, 3n, 4n], new arrow.Int64()),
  }),
)
catalog.append('bench.trades', catalogRows)
benchmark('iceberg/catalog_append', () => catalog.append('bench.trades', catalogRows))

async function benchmarkAsync(name, operation) {
  if (!selected(name)) return
  const asyncWarmups = smoke ? 0 : Math.min(iterations, 5)
  for (let index = 0; index < asyncWarmups; index += 1) await operation()
  const started = performance.now()
  for (let index = 0; index < iterations; index += 1) await operation()
  const elapsed = performance.now() - started
  const rate = Math.round((iterations * 1_000) / elapsed)
  console.log(`${name}: ${rate.toLocaleString('en-US')} operations/second`)
}

async function* asyncRecordGenerator() {
  yield* records
}

async function finish() {
  await benchmarkAsync('records/overwrite_async_records_commit_rows', async () => {
    const handle = stored()
    await handle.overwriteRecords(
      asyncRecordGenerator(),
      committed(handle, 'overwrite'),
    )
  })
  fs.rmSync(root, { recursive: true, force: true })
}

finish().catch((error) => {
  fs.rmSync(root, { recursive: true, force: true })
  console.error(error)
  process.exitCode = 1
})
