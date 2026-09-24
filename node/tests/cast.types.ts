import { ArrowCastPlan, ChunkedSerie, Field, Serie, fields, type ArrowCastOptions } from '..'
import type { Schema as ArrowSchema, Table as ArrowTable } from 'apache-arrow'

declare const schema: ArrowSchema
declare const table: ArrowTable
const root: Field = Field.from('row: struct<id: int64> not null')

const plan: ArrowCastPlan = ArrowCastPlan.compile(
  fields.int32('id'),
  fields.int64('id'),
  { nullability: 'strict' },
)
const fromSchema: ArrowCastPlan = ArrowCastPlan.compile(schema, root)
const fromTable: ArrowCastPlan = ArrowCastPlan.compile(table, 'row: struct<id: int64> not null')
const cast: Serie = plan.apply(Serie.fromScalars(fields.int32('id'), [1]))
const chunked: ChunkedSerie = plan.apply(
  ChunkedSerie.fromSerie(Serie.fromScalars(fields.int32('id'), [1])),
)
const source: Field = plan.source
const target: Field = plan.target
const options: Readonly<Required<ArrowCastOptions>> = plan.options
const identity: boolean = plan.isIdentity
plan.preflight()

// @ts-expect-error the compiled answers are read, never written
plan.options = { safe: true, nullability: 'default', representation: 'value' }
// @ts-expect-error `safe` is a boolean
ArrowCastPlan.compile(fields.int32('id'), fields.int64('id'), { safe: 'yes' })
// @ts-expect-error the private native bridges are hidden
ArrowCastPlan._compileNative

void fromSchema
void fromTable
void cast
void chunked
void source
void target
void options
void identity
