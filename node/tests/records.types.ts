import { RecordBatch as ArrowRecordBatch, Table as ArrowTable } from 'apache-arrow'
import { Buffer } from 'node:buffer'

import {
  BatchReader,
  Filter,
  Plan,
  Selector,
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
const name: string = options.name
const selector: Selector = options.select
const filter: Filter = options.filter
const mergeBy: Selector = options.mergeBy
const plan: Plan = options.plan
const partitionPairs: Array<[string, string]> = options.partitionPairs()
const safe: boolean = options.safe
const batchRowSize: number | null = options.batchRowSize
const maxRowSize: number | null = options.maxRowSize
const maxByteSize: number | null = options.maxByteSize
const commitRowSize: number | null = options.commitRowSize
const level: number = options.level
const blockCodec: string | null = options.blockCodec
const syncMarker: Buffer | null = options.syncMarker
options.name = 'trade'
options.field = Field.from('row: struct<id: int64> not null')
options.field = null
options.select = 'id'
options.select = ['id']
options.select = new Selector('id')
options.filter = 'id > 1'
options.filter = new Filter('id > 1')
options.mergeBy = ['id']
options.plan = 'select id where id > 1'
options.plan = new Plan('select id')
const withSelect: RecordOptions = options.withSelect(['id'])
const withFilter: RecordOptions = options.withFilter('id > 1')
const withPlan: RecordOptions = options.withPlan('select id')
options.safe = true
options.batchRowSize = 1024
options.batchRowSize = null
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
  .withName('trade')
  .withSelect(['id'])
  .withFilter('id > 1')
  .withMergeBy(['id'])
  .withPlan('select id')
  .withSafe(false)
  .withBatchRowSize(512)
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

const merging: RecordOptions = options.withMergeBy(['id'])
const matchKey: string[] = merging.mergeBy.names
handle.overwriteArrowReader(source)
handle.appendArrowReader(BatchReader.from(arrowTable), options)
handle.mergeArrowReader(BatchReader.from(arrowBatch), merging)
handle.overwriteArrowTable(arrowTable, options)
handle.appendArrowTable(arrowTable, named)
handle.mergeArrowTable(arrowTable, merging)
handle.overwriteArrowBatch(arrowBatch, options)
handle.appendArrowBatch(arrowBatch, named)
handle.mergeArrowBatch(arrowBatch, merging)

// The text row's line and its entries answer text; the ranges stand beside them.
import { TextLine, TextOptions } from '..'

const textLine: TextLine = new TextLine(0, '58=caf\u00e9|10=0|', ['FIX.4.4', null])
const lineFromBytes: TextLine = new TextLine(1, Buffer.from('58=caf\u00e9|10=0|'))
const lineUnderOptions: TextLine = new TextLine(2, '58=caf\u00e9|10=0|', null, new TextOptions())
const lineBody: string = textLine.body
const lineCaptures: Array<string | null> = lineFromBytes.captures
const decoded: number = textLine.decodedByteSize
const lineMtime: bigint | null = lineUnderOptions.mtime
const lineBodytype: string = lineUnderOptions.bodytype
const lineIdentity: string = textLine.curruuid
const lineCross: string = textLine.crossuuid
const lineCrosscode: string = textLine.crosscode
const lineHashcode: bigint = textLine.currhashcode
const lineCrosshash: bigint = textLine.crosshashcode
const lineUnix: bigint = textLine.currunix
void [lineMtime, lineBodytype, lineIdentity, lineCross, lineCrosscode, lineHashcode, lineCrosshash, lineUnix]
// A located read answers one identifier at both; a read under a name answers
// it at `sourceuri` alone.
const lineSourceuri: string | null = textLine.sourceuri
const lineSourceurl: string | null = textLine.sourceurl
void [lineSourceuri, lineSourceurl]
const entry = textLine.getEntryByPath('58')
if (entry !== null) {
  const key: string = entry.key
  const value: string = entry.value
  const keyBytes: Buffer = entry.keyBytes
  const valueBytes: Buffer = entry.valueBytes
  void [key, value, keyBytes, valueBytes]
}
// @ts-expect-error a body is text or bytes, never a number
new TextLine(2, 5)
// @ts-expect-error the body is a string, not a Buffer
const bodyBytes: Buffer = textLine.body
void [lineBody, lineCaptures, decoded, bodyBytes]
