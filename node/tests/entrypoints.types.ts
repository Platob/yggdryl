import { RecordBatch, Table, tableFromJSON, tableToIPC } from 'apache-arrow'

import {
  BatchReader,
  Field,
  IOBase,
  RecordOptions,
  type RowSource,
} from '..'

const handle: IOBase = IOBase.fromBytes()
const table: Table = tableFromJSON([{ id: 1, venue: 'XNAS' }])
const batch: RecordBatch = table.batches[0]
const options: RecordOptions = handle.recordOptions()
options.schema = Field.from('row: struct<id: int32> not null')

const read: BatchReader = handle.readArrow()
const readWithOptions: BatchReader = handle.readArrow(options)
const readNamed: BatchReader = handle.readArrow('application/vnd.apache.arrow.stream')

// Every synchronous source returns nothing at all.
const written: void = handle.writeArrow(table)
const writtenBatch: void = handle.writeArrow(batch)
const writtenReader: void = handle.writeArrow(BatchReader.from(table))
const writtenBytes: void = handle.writeArrow(tableToIPC(table))
const writtenRecords: void = handle.writeArrow([{ id: 1, venue: 'XNAS' }])
const writtenColumns: void = handle.writeArrow({ id: [1, 2], venue: ['a', 'b'] })
const writtenWithOptions: void = handle.writeArrow(table, options)
const appended: void = handle.appendArrow(table)
const appendedRecords: void = handle.appendArrow([{ id: 2 }], options)

// An async source is the one shape whose call has to be awaited.
async function* pages(): AsyncIterable<Table> {
  yield table
}
const pending: Promise<void> = handle.writeArrow(pages())
const pendingAppend: Promise<void> = handle.appendArrow(pages())

const source: RowSource = table
const sources: RowSource = [table, batch]
