'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { dtype, field } = yggdryl

const INTEGERS = [
  { fieldClass: field.Int8, name: 'int8' },
  { fieldClass: field.Int16, name: 'int16' },
  { fieldClass: field.Int32, name: 'int32' },
  { fieldClass: field.Int64, name: 'int64' },
  { fieldClass: field.UInt8, name: 'uint8' },
  { fieldClass: field.UInt16, name: 'uint16' },
  { fieldClass: field.UInt32, name: 'uint32' },
  { fieldClass: field.UInt64, name: 'uint64' },
]

for (const { fieldClass, name } of INTEGERS) {
  test(`${name} field pairs a name with the type`, () => {
    const column = new fieldClass('id', false)
    assert.equal(column.name(), 'id')
    assert.equal(column.dataType().name(), name)
    assert.equal(column.isNullable(), false)
    assert.equal(new fieldClass('maybe').isNullable(), true) // nullable by default
  })
}

const OPTIONALS = [
  { fieldClass: field.OptionalInt8, name: 'int8' },
  { fieldClass: field.OptionalInt16, name: 'int16' },
  { fieldClass: field.OptionalInt32, name: 'int32' },
  { fieldClass: field.OptionalInt64, name: 'int64' },
  { fieldClass: field.OptionalUInt8, name: 'uint8' },
  { fieldClass: field.OptionalUInt16, name: 'uint16' },
  { fieldClass: field.OptionalUInt32, name: 'uint32' },
  { fieldClass: field.OptionalUInt64, name: 'uint64' },
]

for (const { fieldClass, name } of OPTIONALS) {
  test(`optional ${name} field`, () => {
    const score = new fieldClass('score')
    assert.equal(score.name(), 'score')
    assert.equal(score.isNullable(), true)
    assert.equal(score.dataType().name(), 'optional')
    assert.equal(score.dataType().valueType().name(), name)
  })
}

test('binary field', () => {
  const payload = new field.Binary('payload')
  assert.equal(payload.name(), 'payload')
  assert.equal(payload.isNullable(), true)
  assert.equal(payload.dataType().name(), 'binary')
  assert.equal(new field.Binary('id', false).isNullable(), false)
})

test('optional binary field', () => {
  const payload = new field.OptionalBinary('payload')
  assert.equal(payload.name(), 'payload')
  assert.equal(payload.dataType().name(), 'optional')
  assert.equal(payload.dataType().valueType().name(), 'binary')
})

test('null field', () => {
  const gap = new field.Null('gap')
  assert.deepEqual([gap.name(), gap.dataType().name(), gap.isNullable()], ['gap', 'null', true])
})

test('union field', () => {
  const union = new dtype.Int64().optional().storage()
  const value = new field.Union('value', union)
  assert.equal(value.name(), 'value')
  assert.equal(value.isNullable(), true)
  assert.equal(value.dataType().arrowFormat(), '+us:0,1')
})
