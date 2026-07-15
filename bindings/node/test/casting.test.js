'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { I32Scalar, I32Serie, U8Scalar, F64Scalar, Utf8Scalar, BinaryScalar } = yggdryl.types

test('numeric scalar cast', () => {
  assert.equal(new I32Scalar(300).toI64().value, '300') // i64 crosses as a string
  assert.equal(new I32Scalar(65).toU8().value, 65)
  assert.equal(new I32Scalar(300).toF64().value, 300.0)
  assert.equal(new U8Scalar(255).toI32().value, 255)
})

test('out-of-range cast throws', () => {
  assert.throws(() => new I32Scalar(300).toU8(), /out of range/)
  assert.throws(() => new I32Scalar(-1).toU8(), /out of range/)
})

test('null casts to null', () => {
  assert.ok(new I32Scalar().toI64().isNull)
  assert.ok(new I32Scalar().toF64().isNull)
  assert.ok(new I32Scalar().toUtf8().isNull)
})

test('serie cast preserves nulls', () => {
  const wide = new I32Serie([1, null, 3]).toI64()
  assert.deepEqual(wide.toOptions(), ['1', null, '3'])
  assert.equal(wide.dataType.name, 'i64')
  const floats = new I32Serie([1, null, 3]).toF64()
  assert.deepEqual(floats.toOptions(), [1.0, null, 3.0])
  assert.throws(() => new I32Serie([1, 300]).toU8())
})

test('utf8 and binary bridges (round trips)', () => {
  assert.equal(new I32Scalar(42).toUtf8().value, '42')
  assert.equal(new Utf8Scalar('42').toI32().value, 42)
  assert.equal(new I32Scalar(-7).toUtf8().toI32().value, -7)
  assert.throws(() => new Utf8Scalar('nope').toI32())

  const b = new I32Scalar(-7).toBinary()
  assert.ok(b.typeName === 'binary' && b.value.length === 4)
  assert.equal(new BinaryScalar(b.value).toI32().value, -7)
  assert.throws(() => new BinaryScalar(Buffer.from([1])).toI32()) // width mismatch
})

test('float bridges', () => {
  assert.equal(new F64Scalar(1.5).toUtf8().value, '1.5')
  assert.equal(new Utf8Scalar('1.5').toF64().value, 1.5)
})
