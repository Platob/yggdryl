import { RecordBatch, Table, tableFromJSON } from 'apache-arrow'

import {
  BatchReader,
  Field,
  IOBase,
  RecordOptions,
  type RecordSource,
  type StructRecord,
  type IOMode,
} from '..'

const handle: IOBase = IOBase.fromBytes()
const table: Table = tableFromJSON([{ id: 1, venue: 'XNAS' }])
const batch: RecordBatch = table.batches[0]
const reader: BatchReader = BatchReader.from(table)
const options: RecordOptions = handle.recordOptions()
options.field = Field.from('row: struct<id: int32> not null')
const merging: RecordOptions = options.withMergeByNames(['id'])
const overIOMode: IOMode = 'overwrite'

const read: BatchReader = handle.readArrowReader()
const readWithOptions: BatchReader = handle.readArrowReader(options)
const readNamed: BatchReader = handle.readArrowReader(
  'application/vnd.apache.arrow.stream',
)

const overwriteReader: void = handle.overwriteArrowReader(reader, options)
const appendReader: void = handle.appendArrowReader(BatchReader.from(table), options)
const mergeReader: void = handle.mergeArrowReader(BatchReader.from(table), merging)
const writeReader: void = handle.writeArrowReader(
  BatchReader.from(table),
  overIOMode,
  options,
)

const overwriteTable: void = handle.overwriteArrowTable(table, options)
const appendTable: void = handle.appendArrowTable(table, options)
const mergeTable: void = handle.mergeArrowTable(table, merging)
const writeTable: void = handle.writeArrowTable(table, 'append', options)

const overwriteBatch: void = handle.overwriteArrowBatch(batch, options)
const appendBatch: void = handle.appendArrowBatch(batch, options)
const mergeBatch: void = handle.mergeArrowBatch(batch, merging)
const writeBatch: void = handle.writeArrowBatch(batch, 'merge', merging)

const record: StructRecord = { id: 1, venue: 'XNAS' }
const records: RecordSource = [record]
const overwriteRecords: void = handle.overwriteRecords(records, options)
const appendRecords: void = handle.appendRecords(record, options)
const mergeRecords: void = handle.mergeRecords(records, merging)
const writeRecords: void = handle.writeRecords(records, 'overwrite', options)
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
const pendingOverwrite: Promise<void> = handle.overwriteRecords(pages(), options)
const pendingAppend: Promise<void> = handle.appendRecords(pages(), options)
const pendingMerge: Promise<void> = handle.mergeRecords(pages(), merging)
const pendingWrite: Promise<void> = handle.writeRecords(pages(), 'append', options)

// @ts-expect-error the mode is required and precedes options
handle.writeArrowReader(BatchReader.from(table), options)
// @ts-expect-error the public mode vocabulary is closed
handle.writeArrowTable(table, 'replace')
// @ts-expect-error the public mode vocabulary is closed
handle.writeArrowBatch(batch, 'upsert')
// @ts-expect-error records use the same closed mode vocabulary
handle.writeRecords(records, 'update')
