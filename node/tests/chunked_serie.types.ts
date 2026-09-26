import {
  BatchReader,
  ChunkedSerie,
  DataType,
  Field,
  Scalar,
  Serie,
  SerieReader,
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
declare const serie: Serie
const root: Field = Field.from('row: struct<id: int64> not null')
const unsafely: ArrowCastOptions = { safe: false }

// Every door answers a chunked serie: of held columns, or of Arrow JS values
// one chunk per Data or per batch.
const empty: ChunkedSerie = ChunkedSerie.empty('id: int64')
const typedEmpty: ChunkedSerie = ChunkedSerie.empty(fields.int64('id'))
const one: ChunkedSerie = ChunkedSerie.fromSerie(serie)
const many: ChunkedSerie = ChunkedSerie.fromSeries([serie, serie])
const cast: ChunkedSerie = ChunkedSerie.fromSeries(new Set([serie]), 'id: int64', unsafely)
const arrays: ChunkedSerie = ChunkedSerie.fromArrowArray(vector)
const wide: ChunkedSerie = ChunkedSerie.fromArrowArray(vector, fields.int64('id'), {
  representation: 'bits',
})
const batches: ChunkedSerie = ChunkedSerie.fromArrowBatch(table, root, unsafely)
const single: ChunkedSerie = ChunkedSerie.fromArrowBatch(batch)
const drained: ChunkedSerie = ChunkedSerie.fromArrowReader(reader, root)

// The chunks are series, and the rows read across them.
const field: Field = batches.field
const dtype: DataType = batches.dtype
const count: number = batches.numChunks
const length: number = batches.length
const chunks: Serie[] = batches.chunks
const chunk: Serie | null = batches.chunk(0)
const nulls: number = batches.nullCount()
const isEmpty: boolean = batches.isEmpty()
const isNull: boolean = batches.isNull(0)
const row: Scalar = batches.scalar(0)
const maybe: Scalar | null = batches.at(0)
const rows: Scalar[] = batches.rows()
const values: unknown[] = batches.asJs({ maxDepth: 4 })
const json: unknown[] = batches.toJSON()
const iter: IterableIterator<Scalar> = batches[Symbol.iterator]()
for (const each of batches) {
  const value: Scalar = each
  void value
}

// Selections answer chunked series, and the join answers one serie.
const window: ChunkedSerie = batches.slice(0, 1)
const child: ChunkedSerie | null = batches.child('id')
const childAt: ChunkedSerie | null = batches.childAt(0)
const children: ChunkedSerie[] = batches.children()
const items: ChunkedSerie | null = batches.items()
const reached: ChunkedSerie | null = batches.getChildByPath('id')
batches.pushChunk(serie)
batches.pushChunk(serie, unsafely)
const joined: Serie = batches.intoSerie()
const recast: ChunkedSerie = batches.cast('id: int64', unsafely)
const typed: ChunkedSerie = batches.cast(DataType.from('int64'))
const copy: ChunkedSerie = batches.clone()

// Arrow goes back out one Data or one batch per chunk.
const back: ArrowVector = batches.intoArrowArray()
const stream: BatchReader = batches.intoArrowReader()
const out: ArrowTable = batches.intoArrowTable()
const held: SerieReader = SerieReader.fromChunked(batches)

// Rows compare against a chunked serie's or a serie's.
const same: boolean = batches.equals(one)
const sameRows: boolean = batches.equals(serie)
const order: number = batches.compare(serie)
const text: string = batches.toString()

// A chunked serie narrows by its class, and is built through a static.
declare const anything: unknown
if (anything instanceof ChunkedSerie) {
  const narrowed: ChunkedSerie = anything
  void narrowed
}
// @ts-expect-error a chunked serie is built through a static
new ChunkedSerie()
// @ts-expect-error a chunk is a Serie, never a JavaScript value
ChunkedSerie.fromSeries([[1n]])
// @ts-expect-error an Arrow door takes an Arrow Vector, not a JavaScript array
ChunkedSerie.fromArrowArray([0])
// @ts-expect-error a cast option is `safe` or `representation`
ChunkedSerie.fromArrowArray(vector, 'id: int64', { strict: true })
// @ts-expect-error a pushed chunk is a Serie
batches.pushChunk([1n])
// @ts-expect-error the chunks are a getter, not a mutable slot
batches.chunks = []
// @ts-expect-error equality takes a chunked serie or a serie
batches.equals([1n])
// @ts-expect-error a held stream of chunks takes a ChunkedSerie
SerieReader.fromChunked(serie)
// @ts-expect-error the private native bridges are hidden
ChunkedSerie._fromArrowArrayIpcNative
// @ts-expect-error the private native bridges are hidden
batches._chunksNative

void [empty, typedEmpty, one, many, cast, arrays, wide, single, drained, field, dtype, count,
  length, chunks, chunk, nulls, isEmpty, isNull, row, maybe, rows, values, json, iter, window,
  child, childAt, children, items, reached, joined, recast, typed, copy, back, stream, out, held,
  same, sameRows, order, text]
