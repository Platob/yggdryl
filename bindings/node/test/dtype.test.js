'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { BooleanType, F32Type, I8Type, I32Type, I64Type, U64Type } = yggdryl.dtype

const ALL_NAMES = [
  'I8Type',
  'I16Type',
  'I32Type',
  'I64Type',
  'U8Type',
  'U16Type',
  'U32Type',
  'U64Type',
  'F32Type',
  'F64Type',
  'BooleanType',
]

test('names, widths, and tags', () => {
  assert.equal(new I8Type().name, 'int8')
  assert.equal(new I8Type().byteWidth, 1)
  assert.equal(new I64Type().byteWidth, 8)
  assert.equal(new F32Type().byteWidth, 4)
  assert.equal(new I64Type().primitiveTag, 'i64')
  assert.equal(new U64Type().primitiveTag, 'u64')
})

test('boolean is bit-packed', () => {
  const dt = new BooleanType()
  assert.equal(dt.name, 'boolean')
  assert.equal(dt.byteWidth, null) // bit-packed
  assert.equal(dt.primitiveTag, null) // outside the core numeric tags
})

test('byte round trip and error', () => {
  const dt = new I32Type()
  assert.ok(dt.serializeBytes().equals(Buffer.alloc(0)))
  assert.ok(I32Type.deserializeBytes(dt.serializeBytes()).equals(dt))
  assert.throws(() => I32Type.deserializeBytes(Buffer.from([1])), /carries no parameters/)
})

test('value semantics', () => {
  const a = new I64Type()
  const b = new I64Type()
  assert.ok(a.equals(b))
  assert.equal(a.hashCode(), b.hashCode())
})

test('dtype namespace surface', () => {
  for (const name of ALL_NAMES) {
    assert.ok(yggdryl.dtype[name] !== undefined, name)
  }
})
