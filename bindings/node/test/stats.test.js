'use strict'

// Numeric-analytics reductions (count / sum / mean / min / max) mirrored from the core
// `NumericSerie` capability onto every numeric leaf Serie. Names + semantics match the Python
// binding exactly: count is the NON-NULL element count; sum is 0 over empty/all-null; mean/min/max
// are null over empty/all-null; NaN propagates through sum/mean and is skipped by min/max.

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { I32Serie, F64Serie } = yggdryl.types

// ---------------------------------------------------------------------------------------
// Integer column (I32Serie)
// ---------------------------------------------------------------------------------------

test('int reductions over a column with nulls exclude the nulls', () => {
  const col = new I32Serie([1, null, 2, 6, null])
  assert.equal(col.length, 5) // every slot
  assert.equal(col.nullCount, 2) // the two nulls
  assert.equal(col.count(), 3) // NON-NULL count, distinct from length/nullCount
  assert.equal(col.sum(), 9)
  assert.equal(col.mean(), 3) // 9 / 3
  assert.equal(col.min(), 1)
  assert.equal(col.max(), 6)
})

test('int reductions over a dense column', () => {
  const col = I32Serie.fromValues([-5, 0, 5, 10])
  assert.equal(col.count(), 4)
  assert.equal(col.sum(), 10)
  assert.equal(col.mean(), 2.5)
  assert.equal(col.min(), -5)
  assert.equal(col.max(), 10)
})

// ---------------------------------------------------------------------------------------
// Float column (F64Serie)
// ---------------------------------------------------------------------------------------

test('float reductions over a column with nulls exclude the nulls', () => {
  const col = new F64Serie([1.5, null, 2.5, -4.0])
  assert.equal(col.count(), 3)
  assert.equal(col.sum(), 0.0) // 1.5 + 2.5 - 4.0
  assert.equal(col.mean(), 0.0)
  assert.equal(col.min(), -4.0)
  assert.equal(col.max(), 2.5)
})

// ---------------------------------------------------------------------------------------
// Empty / all-null: sum === 0, count === 0, mean/min/max === null
// ---------------------------------------------------------------------------------------

// An empty / all-null sum is the additive identity — Rust's f64 `Sum` folds from `-0.0`, so it
// crosses as `-0`, which is IEEE-equal to `0` (`-0 === 0`) though `Object.is` distinguishes them.
const isZero = (value) => value === 0

test('empty column reductions', () => {
  const empty = new I32Serie()
  assert.equal(empty.count(), 0)
  assert.ok(isZero(empty.sum()))
  assert.equal(empty.mean(), null)
  assert.equal(empty.min(), null)
  assert.equal(empty.max(), null)
})

test('all-null column reductions', () => {
  const allNull = new F64Serie([null, null, null])
  assert.equal(allNull.length, 3)
  assert.equal(allNull.count(), 0)
  assert.ok(isZero(allNull.sum()))
  assert.equal(allNull.mean(), null)
  assert.equal(allNull.min(), null)
  assert.equal(allNull.max(), null)
})

// ---------------------------------------------------------------------------------------
// NaN: propagates through sum/mean, is skipped by min/max
// ---------------------------------------------------------------------------------------

test('float NaN propagates through sum and mean', () => {
  const col = new F64Serie([1.0, NaN, 3.0])
  assert.equal(col.count(), 3)
  assert.ok(Number.isNaN(col.sum()))
  assert.ok(Number.isNaN(col.mean()))
})

test('float NaN is skipped by min and max', () => {
  const col = new F64Serie([1.0, NaN, 3.0])
  assert.equal(col.min(), 1.0)
  assert.equal(col.max(), 3.0)
})

test('a column of only NaN still reports non-null count and NaN sum/mean', () => {
  const col = new F64Serie([NaN, NaN])
  assert.equal(col.count(), 2) // NaN is a present (non-null) value
  assert.ok(Number.isNaN(col.sum()))
  assert.ok(Number.isNaN(col.mean()))
  // min/max reduce over present elements via f64::min/f64::max, which return the non-NaN
  // operand — with every element NaN there is no non-NaN operand, so the result is NaN.
  assert.ok(Number.isNaN(col.min()))
  assert.ok(Number.isNaN(col.max()))
})
