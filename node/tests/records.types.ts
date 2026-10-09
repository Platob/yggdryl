import { RecordBatch as ArrowRecordBatch, Table as ArrowTable } from 'apache-arrow'
import { Buffer } from 'node:buffer'

import {
  BatchReader,
  ChunkedSerie,
  Filter,
  Plan,
  Selector,
  Serie,
  StreamChunkedSerie,
  Field,
  IOBase,
  IOResult,
  MimeType,
  RecordOptions,
  type BatchSource,
  type RecordOptionsInput,
  type SchemaInput,
  type SerieInput,
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
const commitBatchNum: number | null = options.commitBatchNum
const numThreads: number | null = options.numThreads
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
options.commitBatchNum = 10
options.commitBatchNum = null
options.numThreads = 4
options.numThreads = null
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
  .withCommitBatchNum(10)
  .withNumThreads(4)
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
const handleOptions: TextOptions | RecordOptions = handle.recordOptions()
void handleOptions
const lines: IterableIterator<TextLine> = handle.readTextLines({ startRownum: 1n })
const linesUnder: Iterable<TextLine> = handle.readTextLines(new TextOptions(), { maxRowSize: 1 })
void lines
void linesUnder
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

// The Serie record doors: a generic Serie out, any shape the rows are held in.
declare const heldSerie: Serie
declare const heldChunks: ChunkedSerie
declare const heldStream: StreamChunkedSerie
const serieRead: Serie = handle.readSerie()
const serieReadUnder: Serie = handle.readSerie(options, { maxRowSize: 1 })
const serieReadBag: Serie = handle.readSerie({ numThreads: 2 })
const serieReadNamed: Serie = handle.readSerie(named)
const serieShapes: SerieInput[] = [heldSerie, heldChunks, heldStream, source, arrowTable, arrowBatch, [arrowBatch], new Uint8Array()]
// Every Serie write answers the rows it read, wrote and skipped.
const serieWritten: IOResult = handle.writeSerie(heldSerie)
const serieAppended: IOResult = handle.writeSerie(heldChunks, 'append', options)
const serieMerged: IOResult = handle.writeSerie(heldStream, 'merge', merging, { numThreads: 1 })
const serieOverwrote: IOResult = handle.overwriteSerie(arrowTable)
const serieDeclared: IOResult = handle.overwriteSerie(heldSerie, { field: schema as Field })
const serieAppendedBatch: IOResult = handle.appendSerie(arrowBatch, named)
const serieMergedReader: IOResult = handle.mergeSerie(source, merging)
const serieSkipped: number = serieWritten.skippedRows
// @ts-expect-error a write's mode is one of the closed IOMode words
handle.writeSerie(heldSerie, 'upsert')
// @ts-expect-error rows are a columnar value, never a number
handle.overwriteSerie(7)
// @ts-expect-error the native write bridge is hidden
handle._writeSerieNative
void [serieRead, serieReadUnder, serieReadBag, serieReadNamed, serieShapes]
void [serieWritten, serieAppended, serieMerged, serieOverwrote, serieDeclared, serieAppendedBatch, serieMergedReader, serieSkipped]

// The text row's line and its entries answer text; the ranges stand beside them.
import { TextLine, TextOptions } from '..'

const textLine: TextLine = new TextLine(0n, '58=caf\u00e9|10=0|', ['FIX.4.4', null])
const lineFromBytes: TextLine = new TextLine(1n, Buffer.from('58=caf\u00e9|10=0|'))
const lineUnderOptions: TextLine = new TextLine(2n, '58=caf\u00e9|10=0|', null, new TextOptions())
const lineIndex: bigint = textLine.index
const lineBody: string = textLine.body
const lineCaptures: Array<string | null> = lineFromBytes.captures
const decoded: number = textLine.decodedByteSize
const lineMtime: bigint | null = lineUnderOptions.mtime
const lineBodytype: string = lineUnderOptions.bodytype
const lineIdentity: string = textLine.uuid
const lineCross: string = textLine.crossuuid
const lineCrosscode: string = textLine.crosscode
const lineHashcode: bigint = textLine.hashcode
const lineCrosshash: bigint = textLine.crosshashcode
const lineUnix: bigint = textLine.transunix
const lineSeqnum: bigint = textLine.seqnum
void [lineIndex, lineMtime, lineBodytype, lineIdentity, lineCross, lineCrosscode, lineHashcode, lineCrosshash, lineUnix, lineSeqnum]
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
// @ts-expect-error a physical line index is an unsigned bigint, never a number
new TextLine(2, '58=caf\u00e9|10=0|')
// @ts-expect-error a body is text or bytes, never a number
new TextLine(2n, 5)
// @ts-expect-error the body is a string, not a Buffer
const bodyBytes: Buffer = textLine.body
void [lineBody, lineCaptures, decoded, bodyBytes]
