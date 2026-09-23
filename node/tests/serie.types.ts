import {
  BatchReader,
  Field,
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

void own
void named
void one
void drained
void scalar
void back
void typedBy
void stream
void held
