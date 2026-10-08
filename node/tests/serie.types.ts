import {
  BatchReader,
  ChunkedSerie,
  KeySerie,
  KeySeries,
  Field,
  FixedSizeSerieSerie,
  LargeSerieSerie,
  LargeSerieViewSerie,
  MapSerie,
  Scalar,
  Serie,
  Selector,
  StreamChunkedSerie,
  StreamKeySerie,
  SerieSerie,
  SerieViewSerie,
  WindowSerie,
  SpillOptions,
  StructSerie,
  Term,
  fields,
  type ArrowCastOptions,
  type JoinKeys,
  type JoinOptionsInput,
  type OrderingKey,
  type OrderingKeys,
  type PartitionOptionsInput,
  type SortOptions,
} from '..'
import type {
  RecordBatch as ArrowRecordBatch,
  Table as ArrowTable,
  Vector as ArrowVector,
} from 'apache-arrow'

declare const vector: ArrowVector
declare const table: ArrowTable
declare const batch: ArrowRecordBatch
declare const reader: BatchReader
const root: Field = Field.from('row: struct<id: int64> not null')
const unsafely: ArrowCastOptions = { safe: false }

// Every Arrow door answers a column, its own field's or the one it was cast into.
const own: Serie = Serie.fromArrowArray(vector)
const wide: Serie = Serie.fromArrowArray(vector, fields.int64('id'), unsafely)
const named: Serie = Serie.fromArrowArray(vector, 'id: int64', { representation: 'bits' })
const records: Serie = Serie.fromArrowBatch(table, root, unsafely)
const one: Serie = Serie.fromArrowBatch(batch)
const drained: Serie = Serie.fromArrowReader(reader, root)
const defaults: Serie = Serie.fromDefault(fields.int32('quantity'), 3)
const cast: Serie = wide.cast('id: int32', unsafely)
const scalar: unknown = defaults.intoArrowScalar()
const back: ArrowVector = cast.intoArrowArray()
const rows: ArrowRecordBatch = records.intoArrowBatch()

// A reader yields one record serie per batch and hands its stream back.
const series: StreamChunkedSerie = StreamChunkedSerie.fromArrowReader(reader, root, unsafely)
const typedBy: Field = series.field
for (const serie of series) {
  const landed: Serie = serie
  void landed
}
const stream: BatchReader = series.intoArrowReader()
// A held column is a stream of the one record serie it is.
const held: StreamChunkedSerie = StreamChunkedSerie.fromSerie(records)
// A held chunked column is a stream of one record serie per chunk.
const chunked: StreamChunkedSerie = StreamChunkedSerie.fromChunked(ChunkedSerie.fromSerie(records))

// @ts-expect-error a held stream takes a Serie, not an Arrow table
StreamChunkedSerie.fromSerie(table)
// @ts-expect-error the private native bridge is hidden
StreamChunkedSerie._fromSerieNative
// @ts-expect-error a stream of chunks takes a ChunkedSerie, not a Serie
StreamChunkedSerie.fromChunked(records)
// @ts-expect-error the private native bridge is hidden
StreamChunkedSerie._fromChunkedNative

// @ts-expect-error an Arrow door takes an Arrow Vector, not a JavaScript array
Serie.fromArrowArray([0])
// @ts-expect-error a cast option is `safe` or `representation`
Serie.fromArrowArray(vector, 'id: int64', { strict: true })
// @ts-expect-error `representation` is a closed vocabulary, not any name
wide.cast('id: int32', { representation: 'raw' })
// @ts-expect-error the private native bridges are hidden
Serie._fromArrowArrayIpcNative
// @ts-expect-error a table is a batch door's input; the table door is retired
Serie.fromArrowTable(table)

// The public constructor converts every iterable's JavaScript values.
const run: Serie = new Serie([1, 'two', null])
const generated: Serie = new Serie(new Set([1, 2]))
const emptyRun: Serie = new Serie()
const absentRun: Serie = new Serie(null)
const nativeRows: unknown[] = wide.asJs({ maxDepth: 4 })
const defaultDepth: unknown[] = wide.asJs({ maxDepth: null })
const jsonRows: unknown[] = wide.toJSON()
const iter: IterableIterator<Scalar> = wide[Symbol.iterator]()
const copy: Serie = wide.clone()
const runCopy: Serie = wide.intoRun()
const sliced: Serie = wide.slice(0, 1)
const child: Serie | null = records.child('id')
const childAt: Serie | null = records.childAt(0)
const children: Serie[] = records.children()
const items: Serie | null = records.items()
const reached: Serie | null = records.getChildByPath('id')
const scalarSerie: Serie | null = wide.intoScalar().asSerie()
wide.splice(0, 1, new Set([1n, 2n]))
wide.splice(0, 1)
wide.splice(0, 1, null)
wide.set(0, 1n)
wide.push(2n)
wide.insert(0, 3n)
wide.extend(new Set([4n, 5n]))
wide.resize(10, 0n)
records.setCell('id', 0, 1n)

// Each runtime leaf class narrows a Serie and lends only its own layout.
if (own instanceof SerieSerie) {
  const offsets: number[] = own.offsets
  const range: [number, number] | null = own.range(0)
  const row: Serie | null = own.row(0)
  const values: unknown[] = own.asJs()
  void [offsets, range, row, values]
  // @ts-expect-error a plain serie has no view sizes
  own.sizes
}
if (own instanceof LargeSerieSerie) {
  const offsets: number[] = own.offsets
  const range: [number, number] | null = own.range(0)
  const row: Serie | null = own.row(0)
  void [offsets, range, row]
}
if (own instanceof SerieViewSerie) {
  const offsets: number[] = own.offsets
  const sizes: number[] = own.sizes
  const range: [number, number] | null = own.range(0)
  const row: Serie | null = own.row(0)
  void [offsets, sizes, range, row]
}
if (own instanceof LargeSerieViewSerie) {
  const offsets: number[] = own.offsets
  const sizes: number[] = own.sizes
  const range: [number, number] | null = own.range(0)
  const row: Serie | null = own.row(0)
  void [offsets, sizes, range, row]
}
if (own instanceof FixedSizeSerieSerie) {
  const width: number = own.width
  const range: [number, number] | null = own.range(0)
  const row: Serie | null = own.row(0)
  void [width, range, row]
  // @ts-expect-error a fixed-size serie has no offsets
  own.offsets
}
if (own instanceof MapSerie) {
  const entries: StructSerie = own.entries
  const keys: Serie = own.keys
  const values: Serie = own.values
  const offsets: number[] = own.offsets
  const sorted: boolean = own.keysSorted
  const range: [number, number] | null = own.range(0)
  const row: StructSerie | null = own.row(0)
  void [entries, keys, values, offsets, sorted, range, row]
  // @ts-expect-error layout properties are getters, not mutable slots
  own.keysSorted = true
}
if (records instanceof StructSerie) {
  const names: string[] = records.names
  const without: StructSerie = records.withoutChild('id')
  void [names, without]
}

// Leaf factories are inherited from Serie and may answer another layout.
const inherited: Serie = MapSerie.fromScalars(fields.int64('id'), [1n])
// @ts-expect-error leaf classes are handed out by factories, never constructed
new SerieSerie()
// @ts-expect-error leaf classes are handed out by factories, never constructed
new LargeSerieSerie()
// @ts-expect-error leaf classes are handed out by factories, never constructed
new SerieViewSerie()
// @ts-expect-error leaf classes are handed out by factories, never constructed
new LargeSerieViewSerie()
// @ts-expect-error leaf classes are handed out by factories, never constructed
new FixedSizeSerieSerie()
// @ts-expect-error leaf classes are handed out by factories, never constructed
new MapSerie()
// @ts-expect-error leaf classes are handed out by factories, never constructed
new StructSerie()
// @ts-expect-error the constructor needs an iterable, not a scalar
new Serie(7)
// @ts-expect-error a base Serie does not lend any leaf layout
wide.offsets
// @ts-expect-error paths are text
records.setCell(1, 0, 1n)
// @ts-expect-error extend takes rows, not one row
wide.extend(1n)
// @ts-expect-error asJs forwards a numeric depth
wide.asJs({ maxDepth: 'deep' })
// @ts-expect-error the private layout bridges are hidden
wide._offsetsNative
void [run, generated, emptyRun, absentRun, nativeRows, defaultDepth, jsonRows, iter,
  copy, runCopy, sliced, child, childAt, children, items, reached, scalarSerie, inherited]

void own
void named
void one
void drained
void scalar
void back
void typedBy
void stream
void held
void chunked

// A serie compares against a chunked serie by the rows.
const chunkedRows: boolean = wide.equals(ChunkedSerie.fromSerie(wide))
const chunkedOrder: number = wide.compare(ChunkedSerie.fromSerie(wide))
void chunkedRows
void chunkedOrder

// Ordering, uniqueness and grouping: the options are an object of two
// booleans, indices, masks and keys a Serie or any iterable of values, and an
// `as*` write answers the serie it was called on.
const descending: SortOptions = { descending: true, nullsFirst: null }
const order: Serie = wide.sortIndices(descending)
const ordered: boolean = wide.isSorted()
const unique: boolean = wide.isUnique()
const distinct: number = wide.uniqueCount()
const bytes: number = wide.memorySize()
const sorted: Serie = wide.intoSorted({ nullsFirst: true })
const deduplicated: Serie = wide.intoUnique()
const reversed: Serie = wide.intoReversed()
const taken: Serie = wide.intoTaken([2, 0])
const takenBy: Serie = wide.intoTaken(order)
const filtered: Serie = wide.intoFiltered([true, false])
const groups: KeySeries = wide.partitionBy(Serie.from(['a', 'b']))
const byPaths: KeySeries = records.partitionBy(['id'])
const byPath: KeySeries = records.partitionBy('id')
const chained: Serie = wide.asSorted().asUnique().asReversed().asTaken([0]).asFiltered([true])
const leafChained: StructSerie = records.child('row') as StructSerie
const sameLeaf: StructSerie = leafChained.asSorted()
const window: WindowSerie = wide.window(0, 1)
// @ts-expect-error an ordering option is `descending` or `nullsFirst`
wide.isSorted({ nullsLast: true })
// @ts-expect-error an ordering option is a boolean
wide.intoSorted({ descending: 'yes' })
// @ts-expect-error a window takes an offset and a length
wide.window(0)
// @ts-expect-error the private ordering bridges are hidden
wide._sortIndicesNative

void [order, ordered, unique, distinct, bytes, sorted, deduplicated, reversed, taken, takenBy,
  filtered, groups, byPaths, byPath, chained, sameLeaf, window]

// Windows by key: each `[key, window]`, the window stating its record; a
// stream cuts into one lazy reader per window.
const windows: KeySeries = records.windowBy('id')
const sortedWindows: KeySeries = records.windowBy(['id'], true)
const clearedWindows: KeySeries = records.windowBy(new Selector('id'), null)
const termWindows: KeySeries = records.windowBy([Term.column('id'), 'id as k'])
const windowRecord: Scalar = windows.get(0)!.key
const walk: StreamKeySerie = StreamChunkedSerie.fromSerie(records).windowBy('id', false)
const walkField: Field = walk.field
const walkKeyField: Field = walk.keyField
const step: IteratorResult<KeySerie> = walk.next()
const self: StreamKeySerie = walk[Symbol.iterator]()
for (const opened of walk) {
  const sub: Serie = opened.rows
  const subRecord: Scalar = opened.key
  const nested: KeySeries = sub.windowBy(Term.column('id'))
  for (const serie of sub.intoStream()) {
    const piece: Scalar = serie
    void piece
  }
  void [subRecord, nested]
}
const readerField: Field = held.field
// @ts-expect-error `sorted` is a boolean
records.windowBy('id', 'yes')
// @ts-expect-error `sorted` is a boolean
StreamChunkedSerie.fromSerie(records).windowBy('id', 1)
// @ts-expect-error a key is a Selector, a Term, a text or an array of them
records.windowBy(7)
// @ts-expect-error a reader's record is a getter, not a mutable slot
held.staticValues = null
// @ts-expect-error a serie states no record: only a window does
records.staticValues
// @ts-expect-error the private windowing bridges are hidden
records._windowByNative
// @ts-expect-error the private windowing bridges are hidden
walk._nextNative

void [windows, sortedWindows, clearedWindows, termWindows, windowRecord, walkField,
  walkKeyField, step, self, readerField]

// Partitions by key: a stream cut into `[key, rows]` pairs, each yielded as
// its partition closes.
const partitionOptions: PartitionOptionsInput = { maxOpen: 2, threads: 1, clustered: false }
const clearedPartitionOptions: PartitionOptionsInput = {
  maxOpen: null,
  threads: null,
  clustered: null,
}
const partitions: StreamKeySerie = StreamChunkedSerie.fromSerie(records).partitionBy('id')
const boundedPartitions: StreamKeySerie = StreamChunkedSerie.fromSerie(records).partitionBy(
  [Term.column('id')],
  partitionOptions,
)
const clearedPartitions: StreamKeySerie = StreamChunkedSerie.fromSerie(records).partitionBy(
  new Selector('id'),
  null,
)
const partitionRoot: Field = partitions.field
const partitionStep: IteratorResult<KeySerie> = partitions.next()
const partitionsSelf: StreamKeySerie = partitions[Symbol.iterator]()
for (const partition of boundedPartitions) {
  const key: Scalar = partition.key
  const rows: Serie = partition.rows
  void [key, rows]
}
// @ts-expect-error a bound is a number
StreamChunkedSerie.fromSerie(records).partitionBy('id', { maxOpen: '2' })
// @ts-expect-error `clustered` is a boolean
StreamChunkedSerie.fromSerie(records).partitionBy('id', { clustered: 1 })
// @ts-expect-error a key is a Selector, a Term, a text or an array of them
StreamChunkedSerie.fromSerie(records).partitionBy(7)
// @ts-expect-error the private partitioning bridges are hidden
partitions._nextNative
// @ts-expect-error the private partitioning bridges are hidden
StreamChunkedSerie.fromSerie(records)._partitionByNative

void [clearedPartitionOptions, clearedPartitions, partitionRoot, partitionStep, partitionsSelf]

// Spill: where the rows live; `spill` takes options or the process default.
const resident: number = wide.residentSize()
const spilled: boolean = wide.isSpilled()
const spilledNothing: void = wide.spill()
wide.spill(null)
wide.spill(new SpillOptions({ byteSize: 0 }))
const readerResident: number = held.residentSize()
const readerSpilled: boolean = held.isSpilled()
held.spill(new SpillOptions({ byteSize: 0n }))
// @ts-expect-error spill takes a SpillOptions, not its init object
wide.spill({ byteSize: 0 })
// `asSpilled` answers the value itself, so calls chain; `intoSpilled` a copy -
// a reader's handing its records over and consuming it.
const spilledInPlace: Serie = wide.asSpilled(new SpillOptions({ byteSize: 0 })).asReversed()
const spilledDefault: Serie = wide.asSpilled()
const spilledCopy: Serie = wide.intoSpilled(null)
const recordSpilled: StructSerie = (records.child('row') as StructSerie).asSpilled()
const readerSpilledInPlace: StreamChunkedSerie = held.asSpilled()
const readerSpilledCopy: StreamChunkedSerie = held.intoSpilled(new SpillOptions({ byteSize: 0 }))
// @ts-expect-error asSpilled takes a SpillOptions, not its init object
wide.asSpilled({ byteSize: 0 })
// @ts-expect-error the private spill bridges are hidden
wide._intoSpilledNative
// @ts-expect-error the private spill bridges are hidden
held._asSpilledNative

// A constant column: one value for every row.
const constant: Serie = Serie.lit(fields.utf8('venue'), 'XNAS', 1_000)
const constantOfText: Serie = Serie.lit('price: int64', 7n, 3)
const isConstant: boolean = constant.isLit
// @ts-expect-error the length is a number of rows
Serie.lit('price: int64', 7, 3n)
// @ts-expect-error a constant is a getter, not a mutable slot
constant.isLit = false
// @ts-expect-error the private constant bridge is hidden
Serie._litNative

void [resident, spilled, spilledNothing, readerResident, readerSpilled, spilledInPlace,
  spilledDefault, spilledCopy, recordSpilled, readerSpilledInPlace, readerSpilledCopy, constant,
  constantOfText, isConstant]

// Orderings by key: the clause's text, key texts, records or a Selector.
const declared: string[] | null = records.declaredOrder()
const key: OrderingKey = { term: 'id', descending: true, nulls_first: false }
const keys: OrderingKeys = ['id desc', key]
const orderBy: Serie = records.sortIndicesBy('id desc nulls first')
const orderByKeys: Serie = records.sortIndicesBy(keys)
const orderBySelector: Serie = records.sortIndicesBy(new Selector('id'))
const sortedBy: Serie = records.intoSortBy(key)
const sortedInPlace: StructSerie = (records.child('row') as StructSerie).asSortBy('id')
const streamSorted: StreamChunkedSerie = StreamChunkedSerie.fromSerie(records).intoSorted({ descending: true })
const streamSortedBy: StreamChunkedSerie = StreamChunkedSerie.fromSerie(records).intoSortBy(['id'])
const camelFlag: Serie = records.sortIndicesBy([{ term: 'id', nullsFirst: true }])
const snakeFlag: Serie = records.sortIndicesBy([{ term: 'id', nulls_first: true }])
// @ts-expect-error a key is text, a record or a Selector
records.intoSortBy(7)
// @ts-expect-error the private ordering bridges are hidden
records._intoSortByNative

void [declared, orderBy, orderByKeys, orderBySelector, sortedBy, sortedInPlace, streamSorted,
  streamSortedBy]

// Joins: keys as text, pairs or a mapping; the kind a word; options an object.
const byPair: JoinKeys = [['id', 'trade_id']]
const joinOptions: JoinOptionsInput = {
  coalesce: false,
  suffix: '_r',
  build: 'left',
  prune: null,
  spill: new SpillOptions(),
  pushdownKeys: 100,
}
const joined: Serie = records.joinWith(records, 'id')
const joinedLeft: Serie = records.joinWith(window, byPair, 'left', joinOptions)
const joinedMap: Serie = records.joinWith(records, new Map([['id', 'id']]), null, null)
const joinedObject: Serie = records.joinWith(records, { id: 'id' }, 'left outer join')
const streamJoined: StreamChunkedSerie = StreamChunkedSerie.fromSerie(records).joinWith(
  StreamChunkedSerie.fromSerie(records),
  'id',
  'semi',
)
const streamJoinedHeld: StreamChunkedSerie = StreamChunkedSerie.fromSerie(records).joinWith(
  ChunkedSerie.fromSerie(records),
  ['id'],
)
// @ts-expect-error the build side is `left` or `right`
records.joinWith(records, 'id', 'inner', { build: 'both' })
// @ts-expect-error a held serie joins a Serie or a window
records.joinWith(ChunkedSerie.fromSerie(records), 'id')
// @ts-expect-error the private join bridge is hidden
records._joinWithNative

void [joined, joinedLeft, joinedMap, joinedObject, streamJoined, streamJoinedHeld]
