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

test('value and data type', () => {
  const present = new I64Scalar(7n)
  assert.equal(present.value, 7n) // always present — a plain value
  assert.ok(present.dataType.equals(new I64Type()))
  assert.equal(present.isNull, undefined) // nullability is not a scalar concern
})

test('byte round trip', () => {
  // A scalar serialises to just its value's little-endian bytes (no null flag).
  const present = new U64Scalar(2n ** 63n) // bigint value
  const raw = present.serializeBytes()
  assert.equal(raw.length, 8)
  assert.ok(U64Scalar.deserializeBytes(raw).equals(present))
})

test('64-bit scalars reject out-of-range bigints', () => {
  // A bigint past the 64-bit range throws instead of being truncated by get_i64/get_u64.
  assert.throws(() => new I64Scalar(2n ** 63n), /out of range for int64/)
  assert.throws(() => new U64Scalar(-1n), /out of range for uint64/)
  assert.throws(() => new U64Scalar(2n ** 64n), /out of range for uint64/)
})

test('deserialize errors are guided', () => {
  // The only decode failure is value bytes that don't fit the data type's width.
  assert.throws(() => I64Scalar.deserializeBytes(Buffer.alloc(0)))
  assert.throws(() => I64Scalar.deserializeBytes(Buffer.from([0, 0, 0])))
})

test('float value semantics are bitwise', () => {
  assert.ok(!new F64Scalar(0.0).equals(new F64Scalar(-0.0))) // distinct bits
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
})

test('boolean scalar', () => {
  assert.equal(new BooleanScalar(true).value, true)
  assert.equal(new BooleanScalar(false).value, false)
})

test('scalar namespace surface', () => {
  for (const name of ALL_NAMES) {
    assert.ok(yggdryl.scalar[name] !== undefined, name)
  }
})

test('default scalar', () => {
  assert.ok(I64Scalar.defaultScalar().equals(new I64Scalar(0n)))
  assert.equal(I64Scalar.defaultScalar().value, 0n)
  assert.ok(F64Scalar.defaultScalar().equals(new F64Scalar(0)))
  assert.equal(BooleanScalar.defaultScalar().value, false)
})
