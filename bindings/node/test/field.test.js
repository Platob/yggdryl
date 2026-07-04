'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { dtype, field } = yggdryl

const INTEGERS = [
  { fieldClass: field.Int8Field, name: 'int8' },
  { fieldClass: field.Int16Field, name: 'int16' },
  { fieldClass: field.Int32Field, name: 'int32' },
  { fieldClass: field.Int64Field, name: 'int64' },
  { fieldClass: field.UInt8Field, name: 'uint8' },
  { fieldClass: field.UInt16Field, name: 'uint16' },
  { fieldClass: field.UInt32Field, name: 'uint32' },
  { fieldClass: field.UInt64Field, name: 'uint64' },
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
  { fieldClass: field.OptionalInt8Field, name: 'int8' },
  { fieldClass: field.OptionalInt16Field, name: 'int16' },
  { fieldClass: field.OptionalInt32Field, name: 'int32' },
  { fieldClass: field.OptionalInt64Field, name: 'int64' },
  { fieldClass: field.OptionalUInt8Field, name: 'uint8' },
  { fieldClass: field.OptionalUInt16Field, name: 'uint16' },
  { fieldClass: field.OptionalUInt32Field, name: 'uint32' },
  { fieldClass: field.OptionalUInt64Field, name: 'uint64' },
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
  const payload = new field.BinaryField('payload')
  assert.equal(payload.name(), 'payload')
  assert.equal(payload.isNullable(), true)
  assert.equal(payload.dataType().name(), 'binary')
  assert.equal(new field.BinaryField('id', false).isNullable(), false)
})

test('optional binary field', () => {
  const payload = new field.OptionalBinaryField('payload')
  assert.equal(payload.name(), 'payload')
  assert.equal(payload.dataType().name(), 'optional')
  assert.equal(payload.dataType().valueType().name(), 'binary')
})

test('null field', () => {
  const gap = new field.NullField('gap')
  assert.deepEqual([gap.name(), gap.dataType().name(), gap.isNullable()], ['gap', 'null', true])
})

test('union field', () => {
  const union = new dtype.Int64Type().optional().storage()
  const value = new field.UnionField('value', union)
  assert.equal(value.name(), 'value')
  assert.equal(value.isNullable(), true)
  assert.equal(value.dataType().arrowFormat(), '+us:0,1')
})

const SERIES = [
  { fieldClass: field.Int8SerieField, ty: dtype.Int8SerieType, name: 'int8' },
  { fieldClass: field.Int16SerieField, ty: dtype.Int16SerieType, name: 'int16' },
  { fieldClass: field.Int32SerieField, ty: dtype.Int32SerieType, name: 'int32' },
  { fieldClass: field.Int64SerieField, ty: dtype.Int64SerieType, name: 'int64' },
  { fieldClass: field.UInt8SerieField, ty: dtype.UInt8SerieType, name: 'uint8' },
  { fieldClass: field.UInt16SerieField, ty: dtype.UInt16SerieType, name: 'uint16' },
  { fieldClass: field.UInt32SerieField, ty: dtype.UInt32SerieType, name: 'uint32' },
  { fieldClass: field.UInt64SerieField, ty: dtype.UInt64SerieType, name: 'uint64' },
]

for (const { fieldClass, ty, name } of SERIES) {
  test(`${name} serie field`, () => {
    const scores = new fieldClass('scores')
    assert.equal(scores.name(), 'scores')
    assert.equal(scores.isNullable(), true)
    assert.equal(scores.dataType().name(), 'list')
    assert.equal(scores.dataType().valueType().name(), name)
    assert.equal(new fieldClass('scores', false).isNullable(), false)
    // The data type's factory builds the same field.
    assert.equal(new ty().field('scores').dataType().name(), 'list')
  })
}

test('fields render as name: type', () => {
  assert.equal(new field.Int64Field('id', false).display(), 'id: int64')
  assert.equal(new field.Int64Field('age').display(), 'age: int64?') // a trailing ? when nullable
  assert.equal(String(new field.Utf8Field('name', false)), 'name: utf8') // napi maps display() to toString()
  assert.equal(new field.Int64SerieField('scores', false).display(), 'scores: list<int64>')
})
