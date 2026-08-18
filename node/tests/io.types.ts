import {
  IOBase,
  MediaType,
  Url,
  type BatchReader,
  type LocationInput,
  type PartitionEntry,
  type PartitionFilters,
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
const parent: IOBase | null = handle.parent
const joined: IOBase = handle.joinpath('year=2024', 'month=01')
const joinedArray: IOBase = handle.joinpath(['year=2024', 'month=01'])

const exists: boolean = handle.exists()
const isDir: boolean = handle.isDir()
const isFile: boolean = handle.isFile()

const children: IOBase[] = handle.iterdir()
const privateChildren: IOBase[] = handle.iterdir(true)
const listed: IOBase[] = handle.ls(true, false)
const globbed: IOBase[] = handle.glob('**/*.parquet')
const recursed: IOBase[] = handle.rglob('*.parquet', true)
const iterated: IOBase[] = [...handle]

const partitions: PartitionEntry[] = handle.partitions
const filters: PartitionFilters = { year: '2024' }
const selected: IOBase[] = handle.childrenWhere(filters)
const byMap: IOBase[] = handle.childrenWhere(new Map([['year', '2024']]))
const byEntries: IOBase[] = handle.childrenWhere([['year', '2024']], true)

const bytes: Buffer = handle.readBytes()
const text: string = handle.readText()
const written: number = handle.writeBytes(Buffer.from('AAPL'))
const wroteText: number = handle.writeText('AAPL')
const head: Buffer = handle.pread(0, 6)
const patched: number = handle.pwrite(0, new Uint8Array([65]))
const appended: number = handle.append(Buffer.from('!'))
const copied: number = handle.copyInto(memory)
const asPath: string = handle.toPath()
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
void parent
void joined
void joinedArray
void exists
void isDir
void isFile
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
void head
void patched
void appended
void copied
const lineBatches: BatchReader = handle.readArrowLines('^\\d{4}')
const lineBatchesTuned: BatchReader = handle.readArrowLines('^\\d{4}', {
  batchSize: 512,
  customFields: { venue: 'XNAS', session: 7 },
  timestampCapture: null,
})
const lineBatchesFromMap: BatchReader = handle.readArrowLines('^\\d{4}', {
  customFields: new Map([['venue', 'XNAS']]),
})

void asPath
void printed
void lineBatches
void lineBatchesTuned
void lineBatchesFromMap
