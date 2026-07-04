'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { factory } = yggdryl

test('factory.scalar infers the type from the value', () => {
  // number / bigint -> int64, Buffer -> binary, null -> null, array -> int64 serie.
  const answer = factory.scalar(42)
  assert.equal(answer.dataType().name(), 'int64')
  assert.equal(answer.asI64(), 42n)

  const fromBig = factory.scalar(42n)
  assert.equal(fromBig.dataType().name(), 'int64')

  const blob = factory.scalar(Buffer.from([1, 2, 3]))
  assert.equal(blob.dataType().name(), 'binary')
  assert.deepEqual([...blob.asBytes()], [1, 2, 3])

  const nothing = factory.scalar(null)
  assert.equal(nothing.dataType().name(), 'null')
  assert.ok(nothing.isNull())

  const numbers = factory.scalar([1, 2, 3])
  assert.equal(numbers.dataType().name(), 'list')
  assert.deepEqual(numbers.toArray(), [1n, 2n, 3n])

  // An empty array defaults to the int64 serie.
  assert.equal(factory.scalar([]).dataType().name(), 'list')
})

test('factory.dtype infers the type from the value', () => {
  assert.equal(factory.dtype(42).name(), 'int64')
  assert.equal(factory.dtype(Buffer.from([1])).name(), 'binary')
  assert.equal(factory.dtype(null).name(), 'null')
  assert.equal(factory.dtype([1, 2, 3]).name(), 'list')
})

test('factory.field infers the type and keeps the name', () => {
  const idField = factory.field('id', 42)
  assert.equal(idField.name(), 'id')
  assert.equal(idField.dataType().name(), 'int64')
  assert.ok(idField.isNullable()) // nullable defaults to true

  const payload = factory.field('payload', Buffer.from([1]), false)
  assert.equal(payload.dataType().name(), 'binary')
  assert.ok(!payload.isNullable())

  assert.equal(factory.field('scores', [1, 2, 3]).dataType().name(), 'list')
  assert.equal(factory.field('maybe', null).dataType().name(), 'null')
})

test('unsupported values throw', () => {
  // A fractional number, a string, a boolean, a plain object, and a non-int array
  // have no matching model type.
  for (const value of [1.5, 'text', true, { a: 1 }, ['x']]) {
    assert.throws(() => factory.scalar(value))
    assert.throws(() => factory.dtype(value))
  }
})
