'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { dtype, factory, scalar } = yggdryl

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

test('factory infers a record from a plain object', () => {
  // A plain object -> a struct row; each member runs the same inference.
  const row = factory.scalar({ id: 7, blob: Buffer.from([1]) })
  assert.ok(row instanceof scalar.RecordScalar)
  assert.equal(row.dataType().name(), 'struct')
  assert.deepEqual(row.toJsValue(), { id: 7n, blob: Buffer.from([1]) })

  const structType = factory.dtype({ id: 7, scores: [1, 2] })
  assert.equal(structType.name(), 'struct')
  assert.deepEqual(structType.fieldNames(), ['id', 'scores'])

  const structField = factory.field('row', { id: 7 }, false)
  assert.equal(structField.name(), 'row')
  assert.equal(structField.dataType().name(), 'struct')
  assert.ok(!structField.isNullable())
})

test('factory accepts its own scalar handles', () => {
  // A scalar handle re-wraps as the same class over the same value...
  const answer = factory.scalar(new scalar.Int64Scalar(42n))
  assert.ok(answer instanceof scalar.Int64Scalar)
  assert.equal(answer.value(), 42n)
  assert.ok(factory.scalar(new scalar.NullScalar()) instanceof scalar.NullScalar)
  assert.deepEqual(factory.scalar(new scalar.BinaryScalar(Buffer.from([1]))).value(), Buffer.from([1]))
  assert.deepEqual(factory.scalar(new scalar.Int64Serie([1n, 2n])).toArray(), [1n, 2n])
  assert.deepEqual(factory.scalar(new scalar.RecordScalar({ id: 7 })).toJsValue(), { id: 7n })

  // ...and classifies as its data type for dtype() / field().
  assert.equal(factory.dtype(new scalar.Int64Scalar(42n)).name(), 'int64')
  assert.equal(factory.dtype(new scalar.RecordScalar({ id: 7 })).name(), 'struct')
  assert.equal(factory.field('id', new scalar.Int64Scalar(42n)).dataType().name(), 'int64')
})

test('factory accepts its own data types', () => {
  // A data type handle is the identity for dtype()...
  assert.equal(factory.dtype(new dtype.NullType()).name(), 'null')
  assert.equal(factory.dtype(new dtype.Int64Type()).name(), 'int64')
  assert.equal(factory.dtype(new dtype.BinaryType()).name(), 'binary')
  assert.equal(factory.dtype(new dtype.Int64SerieType()).name(), 'list')
  assert.deepEqual(factory.dtype(new dtype.StructType({ x: 1 })).fieldNames(), ['x'])

  // ...and builds its default scalar for scalar().
  assert.ok(factory.scalar(new dtype.NullType()).isNull())
  assert.equal(factory.scalar(new dtype.Int64Type()).value(), 0n)
  assert.deepEqual(factory.scalar(new dtype.BinaryType()).value(), Buffer.alloc(0))
  assert.deepEqual(factory.scalar(new dtype.Int64SerieType()).toArray(), [])
  // A struct type's default scalar is the null record (the scalar models nullness).
  assert.ok(factory.scalar(new dtype.StructType({ x: 1 })).isNull())

  assert.equal(factory.field('point', new dtype.StructType({ x: 1 })).dataType().name(), 'struct')
})

test('unsupported values throw', () => {
  // A fractional number, a string, a boolean, a non-int array, and an object
  // with a member of no matching model type.
  for (const value of [1.5, 'text', true, ['x'], { bad: 'text' }]) {
    assert.throws(() => factory.scalar(value))
    assert.throws(() => factory.dtype(value))
  }
})
