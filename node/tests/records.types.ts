import { RecordBatch as ArrowRecordBatch, Table as ArrowTable } from 'apache-arrow'

import {
  BatchReader,
  Field,
  IOBase,
  MimeType,
  RecordOptions,
  type BatchSource,
  type RecordOptionsInput,
  type SchemaInput,
} from '..'

const schema: SchemaInput = Field.from('row: struct<id: int64> not null')
const expression: SchemaInput = 'row: struct<id: int64> not null'

const options: RecordOptions = new RecordOptions(MimeType.ARROW_STREAM)
const inferred: RecordOptions = RecordOptions.from('trades.parquet')
const byMedia: RecordOptions = RecordOptions.forMediaType('application/vnd.apache.parquet')
const byMime: RecordOptions = RecordOptions.forMimeType(MimeType.PARQUET)
const named: RecordOptionsInput = 'trades.arrows'

const mime: MimeType = options.mimeType
const declared: Field | null = options.schema
const rootName: string = options.rootName
const safe: boolean = options.safe
const batchSize: number | null = options.batchSize
const maxRowSize: number | null = options.maxRowSize
const maxByteSize: number | null = options.maxByteSize
const level: number = options.level
options.rootName = 'trade'
options.safe = true
options.batchSize = 1024
options.batchSize = null
options.maxRowSize = 10
options.maxRowSize = null
options.maxByteSize = 1024
options.maxByteSize = null
options.level = 9
const chained: RecordOptions = options
  .withSchema(Field.from('row: struct<id: int64> not null'))
  .withRootName('trade')
  .withSafe(false)
  .withBatchSize(512)
  .withMaxRowSize(10)
  .withMaxByteSize(1024)
  .withLevel(1)
const printed: string = chained.toString()

declare const arrowTable: ArrowTable
declare const arrowBatch: ArrowRecordBatch

const fromTable: BatchReader = BatchReader.from(arrowTable)
const fromBatch: BatchReader = BatchReader.from(arrowBatch)
const fromBatches: BatchReader = BatchReader.from([arrowBatch], 'trade')
const fromBytes: BatchReader = BatchReader.fromIpc(new Uint8Array(), 'trade')
const source: BatchSource = fromTable

const readerField: Field = fromTable.field
const consumed: boolean = fromTable.consumed
const ipc: Buffer = fromTable.toIpc()
const asTable: ArrowTable = fromBatch.toTable()
const batches: ArrowRecordBatch[] = [...fromBatches]

const handle = IOBase.fromBytes()
handle.mediaType = MimeType.ARROW_STREAM
const handleOptions: RecordOptions = handle.recordOptions()
const storedField: Field = handle.readArrowField()
const withOptions: Field = handle.readArrowField(named)
const reader: BatchReader = handle.readArrowBatchReader()
const projected: BatchReader = handle.readArrowBatchReader(options)
const byMediaType: BatchReader = handle.readArrowBatchReader(named)

const merging: RecordOptions = options.withMergeByNames(['id'])
const matchKey: string[] = merging.mergeByNames
handle.writeArrowBatchReader(source)
handle.writeArrowBatchReader(arrowTable, options)
handle.writeArrowBatchReader(arrowBatch, merging)
handle.appendArrowBatchReader([arrowBatch, arrowBatch], named)
