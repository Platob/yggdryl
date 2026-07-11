'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { NullType } = yggdryl.dtype
const { NullField } = yggdryl.field
const { NullScalar } = yggdryl.scalar

test('null type', () => {
  const dt = new NullType()
  assert.equal(dt.name, 'null')
  assert.equal(dt.byteWidth, 0) // a null value is zero bytes
  assert.equal(dt.primitiveTag, null) // sui generis — not a primitive
  assert.equal(dt.serializeBytes().length, 0)
  assert.ok(NullType.deserializeBytes(dt.serializeBytes()).equals(dt))
  assert.ok(dt.equals(new NullType()))
  assert.equal(dt.hashCode(), new NullType().hashCode())
})

test('null field', () => {
  const f = new NullField('maybe', true)
  assert.equal(f.name, 'maybe')
  assert.equal(f.nullable, true)
  assert.ok(f.dataType.equals(new NullType()))
  assert.ok(NullField.deserializeBytes(f.serializeBytes()).equals(f))
})

test('null scalar', () => {
  const s = new NullScalar()
  assert.equal(s.value, null) // its value is the null value
  assert.ok(s.dataType.equals(new NullType()))
  assert.equal(s.serializeBytes().length, 0)
  assert.ok(NullScalar.deserializeBytes(s.serializeBytes()).equals(s))
  assert.ok(s.equals(new NullScalar()))
  assert.equal(s.hashCode(), new NullScalar().hashCode())
  assert.ok(NullScalar.defaultScalar().equals(s)) // the default null scalar is the null value
})
