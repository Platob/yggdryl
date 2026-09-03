import {
  DataType,
  IOBase,
  type ByteIterator,
  type IOCursor,
  MediaType,
  RecordOptions,
  TextOptions,
  Timezone,
  Url,
  type BatchReader,
  type BufferedOptions,
  type Field,
  type LocationInput,
  type PartitionEntry,
  type PartitionFilters,
  type ParquetFileStatistics,
  type ParquetGeospatialStatistics,
} from '..'

const location: LocationInput = 'file:///lake'
const handle = new IOBase(location)
const inferred: IOBase = IOBase.from(Url.fromString('file:///lake'))
const rebuilt: IOBase = IOBase.from(handle)
const memory: IOBase = IOBase.fromBytes(Buffer.from('symbol,price\n'))
const empty: IOBase = IOBase.fromBytes()

const url: Url | null = handle.url
const name: string = handle.name
const mediaType: MediaType = handle.mediaType
const size: number = handle.size
const kind: string = handle.kind
const rowSize: number = handle.rowSize
const columnSize: number = handle.columnSize
const parent: IOBase | null = handle.parent
const joined: IOBase = handle.joinpath('year=2024', 'month=01')
const joinedArray: IOBase = handle.joinpath(['year=2024', 'month=01'])

const exists: boolean = handle.exists()
const isDir: boolean = handle.isDir()
const isFile: boolean = handle.isFile()
const isIo: boolean = handle.isIo()
const isAtomic: boolean = handle.isAtomic()
const isTabular: boolean = handle.isTabular()

const children: IOBase[] = [...handle.iterdir()]
const privateChildren: IOBase[] = [...handle.iterdir(true)]
const listed: IOBase[] = [...handle.ls(true, false)]
const globbed: IOBase[] = [...handle.glob('**/*.parquet')]
const recursed: IOBase[] = [...handle.rglob('*.parquet', true)]
const iterated: IOBase[] = [...handle]

const partitions: PartitionEntry[] = handle.partitions
const filters: PartitionFilters = { year: '2024' }
const selected: IOBase[] = [...handle.childrenWhere(filters)]
const byMap: IOBase[] = [...handle.childrenWhere(new Map([['year', '2024']]))]
const byEntries: IOBase[] = [...handle.childrenWhere([['year', '2024']], true)]

const bytes: Buffer = handle.readBytes()
const text: string = handle.readText()
const written: number = handle.writeBytes(Buffer.from('AAPL'))
const wroteText: number = handle.writeText('AAPL')
handle.writeScalar({ id: 1 })
const value: unknown = handle.readScalar()
const typedValue: { id: number } = handle.readScalar<{ id: number }>(
  'row: struct<id: int64 not null> not null',
)
declare class TypedRow {
  static readonly intoStructField: Field
  id: number
}
const classTypedScalar: TypedRow = handle.readScalar<TypedRow>(TypedRow)
const parquetStatistics: ParquetFileStatistics = handle.readParquetStatistics()
const parquetGeospatial: ParquetGeospatialStatistics =
  handle.readParquetGeospatialStatistics('shape')
const head: Buffer = handle.readRangeBytes(0, 6)
const inferredHead: Buffer = handle.readRange(0, 6)
const explicitHeadBytes: Buffer = handle.readRange(0, 6, { text: false })
const headText: string = handle.readRange(0, 6, { text: true })
const byteStream: ByteIterator = handle.pstreamBytes()
const positionedByteStream: ByteIterator = handle.pstreamBytes(7, 4096)
const streamedChunks: Buffer[] = [...byteStream]
const streamedResult: IteratorResult<Buffer> = positionedByteStream.next()
const cursor: IOCursor = handle.cursor(7)
const cursorByteStream: ByteIterator = cursor.streamBytes(4096)
const cursorChunks: Buffer[] = [...cursorByteStream]
const cacheOptions: BufferedOptions = {
  pageSize: 64 * 1024,
  maxBytes: 8 * 1024 * 1024,
  ttlMs: 30_000,
}
const cachedHandle: IOBase = handle.buffered(cacheOptions)
const patched: number = handle.pwrite(0, new Uint8Array([65]))
const appended: number = handle.appendBytes(Buffer.from('!'))
const appendedText: number = handle.append('!')
const appendedView: number = handle.append(new Uint8Array([33]))
const appendedTyped: number = handle.append(new Int16Array([33]))
// @ts-expect-error the explicit native takes bytes alone; `append` infers
handle.appendBytes('!')
// @ts-expect-error a number is not a byte source
handle.append(12)
// @ts-expect-error text is selected by the option object, not by a class
handle.readRange(0, 6, String)
// @ts-expect-error `{ text: true }` answers a string, never a Buffer
const wrongRange: Buffer = handle.readRange(0, 6, { text: true })
const copied: number = handle.copyInto(memory)
const asPath: string = handle.intoPath()
const printed: string = handle.toString()

handle.mkdir()
handle.touch()
handle.unlink()
handle.truncate(0)
handle.flush()

void inferred
void rebuilt
void empty
void url
void name
void mediaType
void size
void kind
void rowSize
void columnSize
void parent
void joined
void joinedArray
void exists
void isDir
void isFile
void isIo
void isAtomic
void isTabular
void children
void privateChildren
void listed
void globbed
void recursed
void iterated
void partitions
void selected
void byMap
void byEntries
void bytes
void text
void written
void wroteText
void value
void typedValue
void parquetStatistics
void parquetGeospatial
void head
void streamedChunks
void streamedResult
void cursorChunks
void cachedHandle
void patched
void appended
void copied
const textOptions: TextOptions = new TextOptions()
textOptions.rowheader = '\\[(?<level>[A-Z]+)\\]'
textOptions.lstrip = '^\\s+'
textOptions.rstrip = '\\s+$'
textOptions.linesep = new Uint8Array([13, 10])
textOptions.linesep = '\\r\\n'
textOptions.linesep = null
textOptions.autotype = true
textOptions.timezone = Timezone.UTC
textOptions.timezone = 'Europe/Paris'
textOptions.timezone = null
const textHeader: string | null = textOptions.rowheader
const textLstrip: string | null = textOptions.lstrip
const textRstrip: string | null = textOptions.rstrip
const textLinesep: Buffer | null = textOptions.linesep
const textAutotype: boolean = textOptions.autotype
const textTimezone: Timezone | null = textOptions.timezone
const lineBatches: BatchReader = handle.readArrowReader(textOptions)
const lineRecords: IterableIterator<Record<string, unknown>> =
  handle.readRecords(textOptions)
const retainedText: IOBase = handle.intoText(textOptions)
handle.overwriteRecords([{ body: Buffer.from('one') }], textOptions)
handle.appendRecords([{ body: Buffer.from('two') }], textOptions)

void lineBatches
void lineRecords
void textHeader
void textLstrip
void textRstrip
void textLinesep
void textAutotype
void textTimezone
void retainedText

const coding: string | null = handle.codec
const encoded: number = handle.compressInto(memory)
const encodedAs: number = handle.compressInto(memory, 'gzip')
const encodedAtLevel: number = handle.compressInto(memory, 'gzip', 9)
const decoded: number = memory.decompressInto(handle)
const decodedAs: number = memory.decompressInto(handle, 'zstd')

void coding
void encoded
void encodedAs
void encodedAtLevel
void decoded
void decodedAs
