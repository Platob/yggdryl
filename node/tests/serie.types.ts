import {
  BatchReader,
  Field,
  FixedSizeSerieSerie,
  LargeSerieSerie,
  LargeSerieViewSerie,
  MapSerie,
  Scalar,
  Serie,
  SerieReader,
  SerieSerie,
  SerieViewSerie,
  StructSerie,
  fields,
  type ArrowCastOptions,
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
const strictly: ArrowCastOptions = { safe: false, nullability: 'strict' }

// Every Arrow door answers a column, its own field's or the one it was cast into.
const own: Serie = Serie.fromArrowArray(vector)
const wide: Serie = Serie.fromArrowArray(vector, fields.int64('id'), strictly)
const named: Serie = Serie.fromArrowArray(vector, 'id: int64', { representation: 'bits' })
const records: Serie = Serie.fromArrowBatch(table, root, strictly)
const one: Serie = Serie.fromArrowBatch(batch)
const drained: Serie = Serie.fromArrowReader(reader, root)
const defaults: Serie = Serie.fromDefault(fields.int32('quantity'), 3)
const cast: Serie = wide.cast('id: int32', strictly)
const scalar: unknown = defaults.intoArrowScalar()
const back: ArrowVector = cast.intoArrowArray()
const rows: ArrowRecordBatch = records.intoArrowBatch()

// A reader yields one record serie per batch and hands its stream back.
const series: SerieReader = SerieReader.fromArrowReader(reader, root, strictly)
const typedBy: Field = series.field
for (const serie of series) {
  const landed: Serie = serie
  void landed
}
const stream: BatchReader = series.intoArrowReader()
// A held column is a stream of the one record serie it is.
const held: SerieReader = SerieReader.fromSerie(records)

// @ts-expect-error a held stream takes a Serie, not an Arrow table
SerieReader.fromSerie(table)
// @ts-expect-error the private native bridge is hidden
SerieReader._fromSerieNative

// @ts-expect-error an Arrow door takes an Arrow Vector, not a JavaScript array
Serie.fromArrowArray([0])
// @ts-expect-error `nullability` is a closed vocabulary, not any name
Serie.fromArrowArray(vector, 'id: int64', { nullability: 'lenient' })
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
