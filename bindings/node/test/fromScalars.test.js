'use strict'

// Tests for the `fromScalars` factory on every exposed Serie wrapper (fixed / decimal / var / null
// and the temporal columns) plus the native `fromDates` JS-Date factories on the timestamp columns.
// The invariant everywhere: rebuilding a column from its own `getScalar(i)` values reproduces it.

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { I32Scalar, I32Serie, Utf8Serie, BinarySerie, NullSerie } = yggdryl.types
const { D128Serie } = yggdryl.decimal
const { Date32Serie, Ts32Serie, Ts64Serie, Ts96Serie } = yggdryl.temporal

// range(n).map(i => col.getScalar(i)) — the array `fromScalars` must round-trip.
const scalarsOf = (col) => Array.from({ length: col.length }, (_, i) => col.getScalar(i))

// ---------------------------------------------------------------------------------------
// fromScalars — one representative wrapper per family round-trips through its own scalars.
// ---------------------------------------------------------------------------------------

test('fixed: fromScalars round-trips a column (nulls included)', () => {
  const col = new I32Serie([1, null, 3, -5])
  assert.ok(I32Serie.fromScalars(scalarsOf(col)).equals(col))
  assert.ok(I32Serie.fromScalars([]).equals(new I32Serie()))
})

test('fixed: a null/undefined array item is the null scalar', () => {
  const col = I32Serie.fromScalars([new I32Scalar(7), null, undefined, new I32Scalar(9)])
  assert.ok(col.length === 4 && col.nullCount === 2)
  assert.deepEqual(col.toOptions(), [7, null, null, 9])
})

test('decimal: fromScalars round-trips at (precision, scale)', () => {
  const col = new D128Serie(20, 2, ['12.34', null, '5.60'])
  const rebuilt = D128Serie.fromScalars(col.precision, col.scale, scalarsOf(col))
  assert.ok(rebuilt.equals(col))
})

test('var: fromScalars round-trips utf8 and binary columns', () => {
  const utf8 = new Utf8Serie(['a', null, 'cd', ''])
  assert.ok(Utf8Serie.fromScalars(scalarsOf(utf8)).equals(utf8))

  const bin = new BinarySerie([Buffer.from([1, 2]), null, Buffer.from([])])
  assert.ok(BinarySerie.fromScalars(scalarsOf(bin)).equals(bin))
})

test('null: fromScalars round-trips a run of nulls', () => {
  const col = new NullSerie(4)
  assert.ok(NullSerie.fromScalars(scalarsOf(col)).equals(col))
  assert.ok(NullSerie.fromScalars([null, null]).equals(new NullSerie(2)))
})

test('temporal: fromScalars round-trips a column (value wrappers, nulls included)', () => {
  // A timestamp column (zoned) and a date column (naive) — getScalar hands back the VALUE wrapper,
  // and a null slot is a JS `null` the factory maps to a null element.
  const ts = new Ts64Serie('ns', 'UTC', ['2024-07-15T12:00:00Z', null, '2000-01-01T00:00:00Z'])
  assert.ok(Ts64Serie.fromScalars(ts.unit, ts.timezone, scalarsOf(ts)).equals(ts))

  const dates = new Date32Serie('d', undefined, ['2024-02-29', null, '1970-01-01'])
  assert.ok(Date32Serie.fromScalars(dates.unit, dates.timezone, scalarsOf(dates)).equals(dates))
})

// ---------------------------------------------------------------------------------------
// fromDates — the native JS-Date factories on the timestamp columns.
// ---------------------------------------------------------------------------------------

test('Ts64Serie.fromDates builds a column matching the JS Dates', () => {
  const d1 = new Date('2024-07-15T12:00:00Z')
  const d2 = new Date('2000-01-01T00:00:00.123Z')
  const col = Ts64Serie.fromDates('ms', 'UTC', [d1, null, d2])

  assert.ok(col.length === 3 && col.nullCount === 1 && col.unit === 'ms' && col.timezone === 'UTC')
  // At the millisecond unit the raw epoch is exactly `Date.getTime()`.
  assert.equal(col.getEpoch(0), BigInt(d1.getTime()))
  assert.equal(col.get(1), null)
  assert.equal(col.getEpoch(2), BigInt(d2.getTime()))
  // The value wrapper bridges back to the same instant (Ts64.toEpochMillis <-> Date.getTime()).
  assert.equal(col.getScalar(0).toEpochMillis(), d1.getTime())
  assert.equal(col.getScalar(2).toEpochMillis(), d2.getTime())
})

test('Ts32Serie / Ts96Serie fromDates re-express the instant at the column unit', () => {
  const d = new Date('2024-07-15T12:00:00Z')

  // Ts32 at seconds: the ms instant truncates to whole seconds (and fits the 32-bit width).
  const c32 = Ts32Serie.fromDates('s', 'UTC', [d, null])
  assert.ok(c32.length === 2 && c32.nullCount === 1)
  assert.equal(c32.getEpoch(0), BigInt(Math.floor(d.getTime() / 1000)))

  // Ts96 at nanoseconds: the ms instant scales up losslessly.
  const c96 = Ts96Serie.fromDates('ns', 'UTC', [d])
  assert.equal(c96.getEpoch(0), BigInt(d.getTime()) * 1000000n)
})
