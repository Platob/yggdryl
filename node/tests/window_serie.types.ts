import { Field, Scalar, Serie, WindowSerie, type SortOptions } from '..'

const prices: Serie = Serie.fromScalars(new Field('price', 'int64', true), [3, null, 1])
const window: WindowSerie = prices.window(0, 3)

// The window's own facts, and the serie it holds.
const length: number = window.length
const offset: number = window.offset
const parent: Serie = window.serie
const field: Field | null = window.field
const empty: boolean = window.isEmpty()
const nulls: number = window.nullCount()
const absent: boolean = window.isNull(1)
const row: Scalar = window.scalar(0)
const maybe: Scalar | null = window.at(9)
const rows: Scalar[] = window.rows()
const values: unknown[] = window.asJs()
for (const value of window) {
  const scalar: Scalar = value
  void scalar
}
const bytes: number = window.memorySize()
const text: string = window.toString()

// The reads answer what the sliced serie answers.
const options: SortOptions = { descending: true }
const ordered: boolean = window.isSorted(options)
const unique: boolean = window.isUnique()
const distinct: number = window.uniqueCount()
const order: Serie = window.sortIndices()
const narrower: WindowSerie = window.window(1, 1)
const whole: Serie = window.intoSerie()
const sorted: Serie = window.intoSorted({ nullsFirst: true })
const deduplicated: Serie = window.intoUnique()
const reversed: Serie = window.intoReversed()
const taken: Serie = window.intoTaken([0])
const filtered: Serie = window.intoFiltered([true, false, true])
const groups: Array<[Scalar, Serie]> = window.partitionBy(['a', 'b', 'a'])
const same: boolean = window.equals(prices)

// The writes go through the serie and never grow or shrink the window; the
// in-place ones answer the window.
window.set(0, 4)
window.fill(5)
window.swap(0, 2)
window.copyFrom(prices.window(0, 3))
window.copyFrom(prices)
window.splice(0, 1, [6])
const chained: WindowSerie = window.asSorted().asReversed().asTaken([2, 1, 0])

// @ts-expect-error a window has no public constructor
new WindowSerie()
// @ts-expect-error a window never shrinks what it views
window.asUnique()
// @ts-expect-error a window never shrinks what it views
window.asFiltered([true])
// @ts-expect-error a window copies from a window or a serie
window.copyFrom([1, 2, 3])
// @ts-expect-error the private window bridges are hidden
window._setNative

void [length, offset, parent, field, empty, nulls, absent, row, maybe, rows, values, bytes, text,
  ordered, unique, distinct, order, narrower, whole, sorted, deduplicated, reversed, taken,
  filtered, groups, same, chained]
