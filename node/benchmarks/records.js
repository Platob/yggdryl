'use strict'

const fs = require('node:fs')
const os = require('node:os')
const { performance } = require('node:perf_hooks')
const path = require('node:path')
const arrow = require('apache-arrow')
const { BatchReader, Field, IOBase, MimeType, fields, iceberg } = require('..')

// A record crossing is far more work than a schema lookup, so this target
// counts in thousands rather than tens of thousands by default.
const iterations = Number.parseInt(process.env.YGGDRYL_BENCH_ITERATIONS ?? '200', 10)
if (!Number.isSafeInteger(iterations) || iterations <= 0) {
  throw new RangeError('YGGDRYL_BENCH_ITERATIONS must be a positive safe integer')
}

function benchmark(name, operation) {
  for (let index = 0; index < Math.min(iterations, 20); index += 1) operation()
  const started = performance.now()
  for (let index = 0; index < iterations; index += 1) operation()
  const elapsed = performance.now() - started
  const rate = Math.round((iterations * 1_000) / elapsed)
  console.log(`${name}: ${rate.toLocaleString('en-US')} operations/second`)
}

const rows = 10_000
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
stream.writeArrowBatchReader(BatchReader.from(ipc))
const parquet = new IOBase(path.join(root, 'trades.parquet'))
parquet.writeArrowBatchReader(ipc, parquet.recordOptions().withSchema(schema))

const memory = IOBase.fromBytes(stream.readBytes())
memory.mediaType = MimeType.ARROW_STREAM

benchmark('records/reader_from_ipc', () => BatchReader.fromIpc(ipc))
benchmark('records/reader_from_arrow_table', () => BatchReader.from(table))
benchmark('records/read_field_ipc', () => memory.readArrowField())
const memoryWanted = memory.recordOptions().withSchema(wanted)
const parquetWanted = parquet.recordOptions().withSchema(wanted)
benchmark('records/read_ipc_to_ipc', () => memory.readArrowBatchReader().toIpc())
benchmark('records/read_ipc_to_arrow', () => memory.readArrowBatchReader().toTable())
benchmark('records/read_ipc_pushdown', () => memory.readArrowBatchReader(memoryWanted).toIpc())
benchmark('records/read_parquet_to_ipc', () => parquet.readArrowBatchReader().toIpc())
benchmark('records/read_parquet_pushdown', () => parquet.readArrowBatchReader(parquetWanted).toIpc())
benchmark('records/write_ipc', () => memory.writeArrowBatchReader(ipc))
benchmark('records/write_parquet', () => parquet.writeArrowBatchReader(ipc, parquet.recordOptions().withSchema(schema)))

// The generic entry points cross a wider boundary: each infers what it was
// handed before the same native write runs. Comparing `write_arrow_ipc` with
// `write_ipc` above is what says how much the inference itself costs, and the
// other rows say what it costs to build Arrow out of something that is not
// Arrow yet.
const records = new Array(rows)
for (let index = 0; index < rows; index += 1) {
  records[index] = { id: ids[index], symbol: symbols[index], venue: venues[index] }
}
const columns = { id: ids, symbol: symbols, venue: venues }

benchmark('records/write_arrow_ipc', () => memory.writeArrow(ipc))
benchmark('records/write_arrow_table', () => memory.writeArrow(table))
benchmark('records/write_arrow_batches', () => memory.writeArrow(table.batches))
benchmark('records/write_arrow_columns', () => memory.writeArrow(columns))
benchmark('records/write_arrow_records', () => memory.writeArrow(records))
benchmark('records/read_arrow_to_arrow', () => memory.readArrow().toTable())

// One commit writes a data file, a manifest, a manifest list, and a metadata
// document, so an Iceberg append is measured separately from a plain write.
const iced = iceberg.assignFieldIds(schema)
const lake = path.join(root, 'lake')
const iceTable = iceberg.Table.create(lake, iced)
iceTable.append(ipc)

benchmark('iceberg/scan_to_ipc', () => iceTable.scan().toIpc())
benchmark('iceberg/scan_pushdown', () =>
  iceTable.scan(fields.struct('row', [iced.dataType.at(0)], { nullable: false })).toIpc(),
)
benchmark('iceberg/data_files', () => iceTable.dataFiles())
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

fs.rmSync(root, { recursive: true, force: true })
