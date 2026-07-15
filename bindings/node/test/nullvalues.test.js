'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { DataType, NullScalar, NullSerie } = yggdryl.types

test('the types namespace exposes the null value classes', () => {
  for (const cls of [NullScalar, NullSerie]) {
    assert.equal(typeof cls, 'function')
  }
})

test('null scalar', () => {
  const s = new NullScalar()
  assert.ok(s.isNull && !s.isValid() && s.value === null)
  assert.ok(s.typeName === 'null' && s.dataType.equals(DataType.null()))
  assert.ok(s.equals(NullScalar.null()) && s.hashCode() === new NullScalar().hashCode())
  assert.equal(s.serializeBytes().length, 0)
  assert.ok(NullScalar.deserializeBytes(s.serializeBytes()).equals(s))
  assert.equal(s.toString(), 'NullScalar()')
  assert.ok(s.field('n').typeName === 'null' && s.field('n').nullable === true)
  assert.ok(s.toSerie().equals(new NullSerie(1)))
})

test('null serie', () => {
  const col = new NullSerie(3)
  assert.ok(col.length === 3 && col.nullCount === 3 && col.hasNulls)
  col.push()
  col.extend(2)
  assert.equal(col.length, 6)
  assert.equal(col.get(0), null)
  assert.ok(col.getScalar(0).equals(new NullScalar()))
  assert.throws(() => col.getScalar(99))
  assert.ok(new NullSerie().isEmpty())
  assert.ok(col.dataType.equals(DataType.null()))
  assert.ok(col.toField('x').nullable === true && col.toField('x').typeName === 'null')
})

test('null serie equality, codec, copy', () => {
  assert.ok(new NullSerie(2).equals(new NullSerie(2)))
  assert.ok(!new NullSerie().equals(new NullSerie(1)))

  const col = new NullSerie(4)
  assert.ok(NullSerie.deserializeBytes(col.serializeBytes()).equals(col))

  const dup = col.copy()
  dup.push()
  assert.ok(col.length === 4 && dup.length === 5)
})
