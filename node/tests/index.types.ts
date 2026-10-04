import { RecordBatch, Table, tableFromJSON } from 'apache-arrow'

import {
  BatchReader,
  Field,
  IOBase,
  IOResult,
  RecordOptions,
  TextOptions,
  type RecordSource,
  type StructRecord,
  type IOMode,
} from '..'

const handle: IOBase = IOBase.fromBytes()
const table: Table = tableFromJSON([{ id: 1, venue: 'XNAS' }])
const batch: RecordBatch = table.batches[0]
const reader: BatchReader = BatchReader.from(table)
// A plain-text handle answers its TextOptions, every other one RecordOptions.
const own: TextOptions | RecordOptions = handle.recordOptions()
if (!(own instanceof RecordOptions)) throw new TypeError('an Arrow stream answers RecordOptions')
const options: RecordOptions = own
options.field = Field.from('row: struct<id: int32> not null')
const merging: RecordOptions = options.withMergeBy(['id'])
const overIOMode: IOMode = 'overwrite'

const read: BatchReader = handle.readArrowReader()
const readWithOptions: BatchReader = handle.readArrowReader(options)
const readNamed: BatchReader = handle.readArrowReader(
  'application/vnd.apache.arrow.stream',
)

const overwriteReader: IOResult = handle.overwriteArrowReader(reader, options)
const appendReader: IOResult = handle.appendArrowReader(BatchReader.from(table), options)
const mergeReader: IOResult = handle.mergeArrowReader(BatchReader.from(table), merging)
const writeReader: IOResult = handle.writeArrowReader(
  BatchReader.from(table),
  overIOMode,
  options,
)

const overwriteTable: IOResult = handle.overwriteArrowTable(table, options)
const appendTable: IOResult = handle.appendArrowTable(table, options)
const mergeTable: IOResult = handle.mergeArrowTable(table, merging)
const writeTable: IOResult = handle.writeArrowTable(table, 'append', options)

const overwriteBatch: IOResult = handle.overwriteArrowBatch(batch, options)
const appendBatch: IOResult = handle.appendArrowBatch(batch, options)
const mergeBatch: IOResult = handle.mergeArrowBatch(batch, merging)
const writeBatch: IOResult = handle.writeArrowBatch(batch, 'merge', merging)

const record: StructRecord = { id: 1, venue: 'XNAS' }
const records: RecordSource = [record]
const overwriteRecords: IOResult = handle.overwriteRecords(records, options)
const appendRecords: IOResult = handle.appendRecords(record, options)
const mergeRecords: IOResult = handle.mergeRecords(records, merging)
const writeRecords: IOResult = handle.writeRecords(records, 'overwrite', options)
const writtenRows: number = writeRecords.writtenRows
// @ts-expect-error a write answers what it did, not nothing
const answersNothing: void = handle.overwriteArrowTable(table, options)
const readRecords: IterableIterator<Record<string, unknown>> = handle.readRecords(options)
class Trade {
  constructor(readonly row: Record<string, unknown>) {}
}
const readTrades: IterableIterator<Trade> = handle.readRecords(Trade, options)
// @ts-expect-error one call accepts one options value
handle.readRecords(options, options)

async function* pages(): AsyncIterable<StructRecord> {
  yield record
}
const pendingOverwrite: Promise<IOResult> = handle.overwriteRecords(pages(), options)
const pendingAppend: Promise<IOResult> = handle.appendRecords(pages(), options)
const pendingMerge: Promise<IOResult> = handle.mergeRecords(pages(), merging)
const pendingWrite: Promise<IOResult> = handle.writeRecords(pages(), 'append', options)

// @ts-expect-error the mode is required and precedes options
handle.writeArrowReader(BatchReader.from(table), options)
// @ts-expect-error the public mode vocabulary is closed
handle.writeArrowTable(table, 'replace')
// @ts-expect-error the public mode vocabulary is closed
handle.writeArrowBatch(batch, 'upsert')
// @ts-expect-error records use the same closed mode vocabulary
handle.writeRecords(records, 'update')
