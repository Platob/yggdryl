'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { D32Scalar, D32Serie, D64Scalar, D64Serie, D128Scalar, D128Serie, D256Serie } =
  yggdryl.decimal

test('the decimal namespace exposes the columnar classes', () => {
  for (const cls of [D32Scalar, D32Serie, D128Scalar, D128Serie, D256Serie]) {
    assert.equal(typeof cls, 'function')
  }
})

// ---------------------------------------------------------------------------------------
// Scalar
// ---------------------------------------------------------------------------------------

test('scalar infers or pins precision/scale', () => {
  const inferred = new D128Scalar('123.45')
  assert.ok(inferred.value === '123.45' && inferred.precision === 5 && inferred.scale === 2)

  const pinned = new D128Scalar('123.45', 20, 2)
  assert.ok(pinned.precision === 20 && pinned.scale === 2)

  for (const nul of [new D128Scalar(), new D128Scalar(null), D128Scalar.null(10, 2)]) {
    assert.ok(nul.isNull && nul.value === null)
  }
})

test('scalar value identity across scale', () => {
  assert.ok(new D128Scalar('2.5', 5, 1).equals(new D128Scalar('2.50', 5, 2)))
  assert.equal(new D128Scalar('2.5', 5, 1).hashCode(), new D128Scalar('2.50', 5, 2).hashCode())
  assert.ok(!new D128Scalar('2.5', 5, 1).equals(new D128Scalar('2.75', 5, 2)))
})

test('scalar precision overflow throws', () => {
  assert.throws(() => new D128Scalar('1.234', 5, 2))
})

test('scalar byte codec (value and null)', () => {
  for (const cls of [D32Scalar, D64Scalar, D128Scalar]) {
    const s = new cls('12.34', 10, 2)
    assert.ok(cls.deserializeBytes(s.serializeBytes()).equals(s))
    assert.ok(cls.deserializeBytes(cls.null(10, 2).serializeBytes()).equals(cls.null(10, 2)))
  }
})

// ---------------------------------------------------------------------------------------
// Serie
// ---------------------------------------------------------------------------------------

test('serie construction and access', () => {
  const col = new D128Serie(20, 2, ['123.45', null, '6'])
  assert.ok(col.length === 3 && col.nullCount === 1 && col.hasNulls)
  assert.ok(col.precision === 20 && col.scale === 2)
  assert.ok(col.get(0) === '123.45' && col.get(1) === null)
  assert.deepEqual(col.toOptions(), ['123.45', null, '6.00']) // re-expressed at scale 2

  const dense = D128Serie.fromValues(10, 2, ['1', '2'])
  assert.ok(dense.nullCount === 0)
  assert.deepEqual(dense.toOptions(), ['1.00', '2.00'])
  assert.ok(new D128Serie(10, 2).isEmpty())
})

test('serie mutation and fit', () => {
  const col = new D64Serie(10, 2, ['1.00', null])
  col.push('3')
  col.set(1, '2.50')
  assert.deepEqual(col.toOptions(), ['1.00', '2.50', '3.00'])
  assert.ok(col.getScalar(0).equals(new D64Scalar('1.00', 10, 2)))
  assert.throws(() => col.set(0, '1.234')) // does not fit scale 2
  assert.throws(() => col.set(99, '0')) // out of range
})

test('serie byte codec including cleared-null canonical identity', () => {
  const col = new D128Serie(20, 2, ['123.45', null, '6'])
  col.set(1, '0.01') // clears the last null
  assert.ok(D128Serie.deserializeBytes(col.serializeBytes()).equals(col))
})

test('serie copy is independent', () => {
  const original = new D256Serie(10, 0, ['1', '2'])
  const dup = original.copy()
  dup.push('3')
  assert.ok(original.length === 2 && dup.length === 3)
})
