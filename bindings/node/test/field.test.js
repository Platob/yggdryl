'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { BooleanField, I32Field, I64Field } = yggdryl.field
const { I64Type } = yggdryl.dtype

const ALL_NAMES = [
  'I8Field',
  'I16Field',
  'I32Field',
  'I64Field',
  'U8Field',
  'U16Field',
  'U32Field',
  'U64Field',
  'F32Field',
  'F64Field',
  'BooleanField',
]

test('name, nullable, and data type', () => {
  const f = new I64Field('id', false)
  assert.equal(f.name, 'id')
  assert.equal(f.nullable, false)
  assert.ok(f.dataType.equals(new I64Type()))
  assert.equal(f.dataType.name, 'int64')

  // nullable defaults to false.
  assert.equal(new I32Field('count').nullable, false)
  assert.equal(new BooleanField('flag', true).nullable, true)
})

test('byte round trip and errors', () => {
  const f = new I64Field('mesure_€', true) // non-ASCII name
  assert.ok(I64Field.deserializeBytes(f.serializeBytes()).equals(f))
  assert.equal(f.serializeBytes()[0], 1) // nullable flag first
  assert.throws(() => I64Field.deserializeBytes(Buffer.alloc(0)), /nullable flag/)
})

test('value semantics', () => {
  const a = new I64Field('a', true)
  assert.ok(a.equals(new I64Field('a', true)))
  assert.ok(!a.equals(new I64Field('a', false)))
  assert.ok(!a.equals(new I64Field('b', true)))
  assert.equal(a.hashCode(), new I64Field('a', true).hashCode())
})

test('field headers round-trips and is identity-bearing', () => {
  const entries = [{ key: Buffer.from('unit'), value: Buffer.from('ms') }]
  const f = new I64Field('ts', true).withHeaders(entries)
  assert.ok(f.headers[0].value.equals(Buffer.from('ms')))
  assert.equal(new I64Field('ts', true).headers, null)
  // Byte round-trip carries the headers.
  assert.ok(I64Field.deserializeBytes(f.serializeBytes()).equals(f))
  // Headers is part of the field's identity.
  assert.ok(!f.equals(new I64Field('ts', true)))
})

test('field namespace surface', () => {
  for (const name of ALL_NAMES) {
    assert.ok(yggdryl.field[name] !== undefined, name)
  }
})
