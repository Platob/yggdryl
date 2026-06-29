'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const { Binary, Utf8, Field, BinaryScalar, StringScalar } = require('../index.js')

test('binary datatype round-trips', () => {
  const b = new Binary()
  assert.equal(b.name, 'binary')
  assert.equal(b.toString(), 'binary')
  assert.equal(b.isLarge, false)
  assert.equal(b.isUtf8, false)
  assert.equal(new Binary(true).name, 'large_binary')

  assert.ok(Binary.fromMapping(b.toMapping()).equals(b))
  assert.ok(Binary.fromBytes(b.toBytes()).equals(b))
})

test('utf8 datatype and aliases', () => {
  const s = new Utf8()
  assert.equal(s.name, 'string')
  assert.equal(s.isUtf8, true)
  assert.equal(new Utf8(true).name, 'large_string')
  assert.ok(Utf8.fromBytes(Buffer.from('string')).equals(s))
})

test('datatypes round-trip through JSON.stringify / fromJSON', () => {
  for (const value of [new Binary(), new Binary(true), new Utf8(), new Utf8(true)]) {
    const json = JSON.stringify(value)
    const restored = value.constructor.fromJSON(JSON.parse(json))
    assert.ok(restored.equals(value))
  }
  assert.equal(JSON.stringify(new Binary()), '"binary"')
})

test('field round-trips with metadata', () => {
  const field = new Field('payload', new Binary(true), false, { unit: 'bytes' })
  assert.equal(field.name, 'payload')
  assert.ok(field.dataType.equals(new Binary(true)))
  assert.equal(field.nullable, false)
  assert.deepEqual(field.metadata, { unit: 'bytes' })

  assert.ok(Field.fromMapping(field.toMapping()).equals(field))
  assert.ok(Field.fromBytes(field.toBytes()).equals(field))
  assert.ok(Field.fromJSON(JSON.parse(JSON.stringify(field))).equals(field))
})

test('field defaults nullable to true and helpers do not mutate', () => {
  const field = new Field('id', new Utf8())
  assert.equal(field.nullable, true)

  const renamed = field.withName('other').withNullable(false)
  assert.equal(field.name, 'id')
  assert.equal(field.nullable, true)
  assert.equal(renamed.name, 'other')
  assert.equal(renamed.nullable, false)
})

test('binary scalar', () => {
  const scalar = new BinaryScalar(Buffer.from([0, 1, 2]))
  assert.deepEqual([...scalar.value], [0, 1, 2])
  assert.equal(scalar.isNull, false)
  assert.equal(scalar.length, 3)
  assert.ok(scalar.dataType.equals(new Binary()))
  assert.equal(new BinaryScalar().isNull, true)
  assert.equal(BinaryScalar.null().value, null)
  assert.ok(BinaryScalar.fromJSON(JSON.parse(JSON.stringify(scalar))).equals(scalar))
})

test('string scalar', () => {
  const scalar = new StringScalar('yggdryl')
  assert.equal(scalar.value, 'yggdryl')
  assert.equal(scalar.toString(), 'yggdryl')
  assert.ok(scalar.dataType.equals(new Utf8()))
  assert.equal(new StringScalar(null).isNull, true)
  assert.ok(StringScalar.fromJSON(JSON.parse(JSON.stringify(scalar))).equals(scalar))
})
