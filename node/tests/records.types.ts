import { RecordBatch as ArrowRecordBatch, Table as ArrowTable } from 'apache-arrow'
import { Buffer } from 'node:buffer'

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
const optionsClone: RecordOptions = options.clone()
const optionsEqual: boolean = options.equals(optionsClone)
const optionsOrder: number = options.compare(optionsClone)
const optionsHash: bigint = options.stableHash()

const mime: MimeType = options.mimeType
const declared: Field | null = options.field
const rootName: string = options.rootName
const safe: boolean = options.safe
const batchSize: number | null = options.batchSize
const maxRowSize: number | null = options.maxRowSize
const maxByteSize: number | null = options.maxByteSize
const commitRowSize: number | null = options.commitRowSize
const level: number = options.level
const blockCodec: string | null = options.blockCodec
const syncMarker: Buffer | null = options.syncMarker
options.rootName = 'trade'
options.safe = true
options.batchSize = 1024
options.batchSize = null
options.maxRowSize = 10
options.maxRowSize = null
options.maxByteSize = 1024
options.maxByteSize = null
options.commitRowSize = 1_000
options.commitRowSize = null
options.level = 9
const avroOptions = RecordOptions.from('trades.avro')
avroOptions.blockCodec = 'zstandard'
avroOptions.syncMarker = Buffer.from('0123456789abcdef')
avroOptions.syncMarker = null
const avroCopy: RecordOptions = avroOptions
  .withBlockCodec('null')
  .withSyncMarker(Buffer.from('fedcba9876543210'))
const chained: RecordOptions = options
  .withField(Field.from('row: struct<id: int64> not null'))
  .withRootName('trade')
  .withSafe(false)
  .withBatchSize(512)
  .withMaxRowSize(10)
  .withMaxByteSize(1024)
  .withCommitRowSize(1_000)
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
const ipc: Buffer = fromTable.intoIpc()
const asTable: ArrowTable = fromBatch.intoTable()
const batches: ArrowRecordBatch[] = [...fromBatches]

const handle = IOBase.fromBytes()
handle.mediaType = MimeType.ARROW_STREAM
const handleOptions: RecordOptions = handle.recordOptions()
const storedField: Field = handle.readArrowField()
const withOptions: Field = handle.readArrowField(named)
const reader: BatchReader = handle.readArrowReader()
const projected: BatchReader = handle.readArrowReader(options)
const byMediaType: BatchReader = handle.readArrowReader(named)

const merging: RecordOptions = options.withMergeByNames(['id'])
const matchKey: string[] = merging.mergeByNames
handle.overwriteArrowReader(source)
handle.appendArrowReader(BatchReader.from(arrowTable), options)
handle.mergeArrowReader(BatchReader.from(arrowBatch), merging)
handle.overwriteArrowTable(arrowTable, options)
handle.appendArrowTable(arrowTable, named)
handle.mergeArrowTable(arrowTable, merging)
handle.overwriteArrowRecordBatch(arrowBatch, options)
handle.appendArrowRecordBatch(arrowBatch, named)
handle.mergeArrowRecordBatch(arrowBatch, merging)
