'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { dtype, field, scalar } = yggdryl

test('record scalar builds from a plain object', () => {
  const row = new scalar.RecordScalar({
    id: 7,
    big: 42n,
    blob: Buffer.from([1, 2]),
    xs: [1, 2, 3],
    gone: null,
  })
  assert.equal(row.isNull(), false)
  assert.deepEqual(row.fieldNames(), ['id', 'big', 'blob', 'xs', 'gone'])

  // get(name) reads one field as the child class' wire type: number and bigint
  // members both infer to int64, so both read back as BigInt.
  assert.equal(row.get('id'), 7n)
  assert.equal(row.get('big'), 42n)
  assert.deepEqual(row.get('blob'), Buffer.from([1, 2]))
  assert.deepEqual(row.get('xs'), [1n, 2n, 3n])
  assert.equal(row.get('gone'), null)
  assert.equal(row.get('missing'), null) // no such field

  // toJsValue is the whole row as one plain object, in one FFI call.
  assert.deepEqual(row.toJsValue(), {
    id: 7n,
    big: 42n,
    blob: Buffer.from([1, 2]),
    xs: [1n, 2n, 3n],
    gone: null,
  })

  // The empty record is a valid zero-field row.
  assert.deepEqual(new scalar.RecordScalar({}).toJsValue(), {})
})

test('record data type is the inferred struct', () => {
  const structType = new scalar.RecordScalar({ id: 7, blob: Buffer.from([1]) }).dataType()
  assert.equal(structType.name(), 'struct')
  assert.equal(structType.arrowFormat(), '+s')
  assert.equal(structType.byteWidth(), null)
  assert.equal(structType.childCount(), 2)
  assert.deepEqual(structType.fieldNames(), ['id', 'blob'])

  // The struct data type also builds straight from example values.
  const point = new dtype.StructType({ x: 1, y: 2n })
  assert.equal(point.name(), 'struct')
  assert.equal(point.childCount(), 2)
  assert.deepEqual(point.fieldNames(), ['x', 'y'])
})

test('struct field pairs a name with the struct type', () => {
  const point = new dtype.StructType({ x: 1, y: 2 })
  const structField = new field.StructField('point', point)
  assert.equal(structField.name(), 'point')
  assert.ok(structField.isNullable()) // nullable defaults to true
  assert.equal(structField.dataType().name(), 'struct')
  assert.deepEqual(structField.dataType().fieldNames(), ['x', 'y'])
  assert.ok(!new field.StructField('point', point, false).isNullable())
})

test('the null record', () => {
  const row = scalar.RecordScalar.null(new dtype.StructType({ x: 1 }))
  assert.equal(row.isNull(), true)
  assert.deepEqual(row.fieldNames(), ['x']) // the type keeps its fields
  assert.equal(row.get('x'), null)
  assert.equal(row.toJsValue(), null)
})

test('record with a float member reads the number', () => {
  // A fractional member infers float64; a whole member stays int64.
  const row = new scalar.RecordScalar({ id: 7, weight: 1.5 })
  assert.deepEqual(row.fieldNames(), ['id', 'weight'])
  assert.equal(row.dataType().childCount(), 2)
  assert.equal(row.get('id'), 7n)
  assert.equal(row.get('weight'), 1.5) // read back as a number, one FFI call
  assert.deepEqual(row.toJsValue(), { id: 7n, weight: 1.5 })

  // The inferred struct data type carries the float64 member.
  const structType = new dtype.StructType({ id: 7, weight: 1.5 })
  assert.deepEqual(structType.fieldNames(), ['id', 'weight'])
})

test('a member the model cannot infer throws', () => {
  assert.throws(() => new scalar.RecordScalar({ bad: 'text' }), /bad/)
  assert.throws(() => new dtype.StructType({ bad: true }), /bad/)
})
