'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { BooleanScalar, F64Scalar, I64Scalar, U64Scalar, F32Scalar } = yggdryl.scalar
const { I64Type } = yggdryl.dtype

const ALL_NAMES = [
  'I8Scalar',
  'I16Scalar',
  'I32Scalar',
  'I64Scalar',
  'U8Scalar',
  'U16Scalar',
  'U32Scalar',
  'U64Scalar',
  'F32Scalar',
  'F64Scalar',
  'BooleanScalar',
]

test('present and null', () => {
  const present = new I64Scalar(7n)
  assert.equal(present.value, 7n)
  assert.equal(present.isNull, false)
  assert.ok(present.dataType.equals(new I64Type()))

  const nul = new I64Scalar(null)
  assert.equal(nul.value, null)
  assert.equal(nul.isNull, true)
  assert.equal(new I64Scalar().isNull, true) // no-arg constructor
  assert.equal(I64Scalar.null().isNull, true) // factory
})

test('byte round trip present and null', () => {
  const present = new U64Scalar(2n ** 63n) // bigint value
  assert.ok(U64Scalar.deserializeBytes(present.serializeBytes()).equals(present))
  assert.equal(present.serializeBytes()[0], 1) // present flag

  const nul = I64Scalar.null()
  assert.ok(nul.serializeBytes().equals(Buffer.from([0])))
  assert.ok(I64Scalar.deserializeBytes(Buffer.from([0])).equals(nul))
})

test('deserialize errors are guided', () => {
  assert.throws(() => I64Scalar.deserializeBytes(Buffer.alloc(0)), /null flag/)
  assert.throws(() => I64Scalar.deserializeBytes(Buffer.from([2])), /expected 0/)
})

test('float value semantics are bitwise', () => {
  assert.ok(!new F64Scalar(0.0).equals(new F64Scalar(-0.0))) // distinct bits
  assert.ok(!new F64Scalar(1.0).equals(F64Scalar.null()))
  assert.ok(new F64Scalar(NaN).equals(new F64Scalar(NaN))) // same bits
})

test('float32 marshals over f64', () => {
  const s = new F32Scalar(1.5)
  assert.equal(s.value, 1.5)
  assert.ok(F32Scalar.deserializeBytes(s.serializeBytes()).equals(s))
})

test('value semantics', () => {
  const a = new I64Scalar(5n)
  assert.ok(a.equals(new I64Scalar(5n)))
  assert.ok(!a.equals(new I64Scalar(6n)))
  assert.equal(a.hashCode(), new I64Scalar(5n).hashCode())
  assert.ok(I64Scalar.null().equals(I64Scalar.null()))
})

test('boolean scalar', () => {
  assert.equal(new BooleanScalar(true).value, true)
  assert.equal(new BooleanScalar(false).value, false)
  assert.equal(BooleanScalar.null().isNull, true)
})

test('scalar namespace surface', () => {
  for (const name of ALL_NAMES) {
    assert.ok(yggdryl.scalar[name] !== undefined, name)
  }
})
