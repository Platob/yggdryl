'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('../index.js')
const { data } = yggdryl

// The 8-32 bit types carry values as `number`; the 64-bit types as `BigInt`.
const INTEGERS = [
  { ty: data.Int8, field: data.Int8Field, scalar: data.Int8Scalar, optional: data.OptionalInt8Scalar, name: 'int8', format: 'c', width: 1, low: -(2 ** 7), high: 2 ** 7 - 1, wire: (v) => v },
  { ty: data.Int16, field: data.Int16Field, scalar: data.Int16Scalar, optional: data.OptionalInt16Scalar, name: 'int16', format: 's', width: 2, low: -(2 ** 15), high: 2 ** 15 - 1, wire: (v) => v },
  { ty: data.Int32, field: data.Int32Field, scalar: data.Int32Scalar, optional: data.OptionalInt32Scalar, name: 'int32', format: 'i', width: 4, low: -(2 ** 31), high: 2 ** 31 - 1, wire: (v) => v },
  { ty: data.Int64, field: data.Int64Field, scalar: data.Int64Scalar, optional: data.OptionalInt64Scalar, name: 'int64', format: 'l', width: 8, low: -(2n ** 63n), high: 2n ** 63n - 1n, wire: (v) => BigInt(v) },
  { ty: data.UInt8, field: data.UInt8Field, scalar: data.UInt8Scalar, optional: data.OptionalUInt8Scalar, name: 'uint8', format: 'C', width: 1, low: 0, high: 2 ** 8 - 1, wire: (v) => v },
  { ty: data.UInt16, field: data.UInt16Field, scalar: data.UInt16Scalar, optional: data.OptionalUInt16Scalar, name: 'uint16', format: 'S', width: 2, low: 0, high: 2 ** 16 - 1, wire: (v) => v },
  { ty: data.UInt32, field: data.UInt32Field, scalar: data.UInt32Scalar, optional: data.OptionalUInt32Scalar, name: 'uint32', format: 'I', width: 4, low: 0, high: 2 ** 32 - 1, wire: (v) => v },
  { ty: data.UInt64, field: data.UInt64Field, scalar: data.UInt64Scalar, optional: data.OptionalUInt64Scalar, name: 'uint64', format: 'L', width: 8, low: 0n, high: 2n ** 64n - 1n, wire: (v) => BigInt(v) },
]

for (const { ty, field, scalar, optional, name, format, width, low, high, wire } of INTEGERS) {
  test(`${name} data type describes itself`, () => {
    const instance = new ty()
    assert.equal(instance.name(), name)
    assert.equal(instance.arrowFormat(), format)
    assert.equal(instance.byteWidth(), width)
    assert.equal(instance.bitWidth(), width * 8)
  })

  test(`${name} codec round-trips`, () => {
    const instance = new ty()
    for (const value of [low, wire(0), wire(42), high]) {
      const encoded = instance.nativeToBytes(value)
      assert.equal(encoded.length, width)
      assert.equal(instance.nativeFromBytes(encoded), value)
    }
    // Little-endian: the low byte comes first.
    assert.equal(instance.nativeToBytes(wire(1))[0], 1)
    assert.throws(() => instance.nativeFromBytes(Buffer.alloc(width + 1)))
  })

  test(`${name} field pairs a name with the type`, () => {
    const column = new field('id', false)
    assert.equal(column.name(), 'id')
    assert.equal(column.dataType().name(), name)
    assert.equal(column.isNullable(), false)
    assert.equal(new field('maybe').isNullable(), true) // nullable by default
  })

  test(`${name} scalar holds a value or null`, () => {
    const answer = new scalar(wire(42))
    assert.equal(answer.isNull(), false)
    assert.equal(answer.value(), wire(42))
    assert.equal(answer.dataType().name(), name)
    assert.equal(new scalar(low).value(), low)
    assert.equal(new scalar(high).value(), high)

    const missing = scalar.null()
    assert.equal(missing.isNull(), true)
    assert.equal(missing.value(), null)
  })

  test(`${name} accessors convert exactly`, () => {
    const answer = new scalar(wire(42))
    // A small value converts to every numeric target.
    assert.equal(answer.asI8(), 42)
    assert.equal(answer.asI16(), 42)
    assert.equal(answer.asI32(), 42)
    assert.equal(answer.asI64(), 42n)
    assert.equal(answer.asU8(), 42)
    assert.equal(answer.asU16(), 42)
    assert.equal(answer.asU32(), 42)
    assert.equal(answer.asU64(), 42n)
    assert.equal(answer.asF32(), 42)
    assert.equal(answer.asF64(), 42)
    // An integer is never a bool or a str.
    assert.equal(answer.asBool(), null)
    assert.equal(answer.asStr(), null)
    // Null answers null everywhere.
    assert.equal(scalar.null().asI64(), null)
  })

  test(`${name} optional scalar redirects to the inner scalar`, () => {
    const answer = new optional(wire(42))
    assert.equal(answer.isNull(), false)
    assert.equal(answer.value(), wire(42))
    assert.equal(answer.scalar().value(), wire(42))
    assert.equal(answer.asI64(), 42n)

    const union = answer.dataType()
    assert.equal(union.name(), 'union')
    assert.equal(union.arrowFormat(), '+us:0,1')
    assert.equal(union.childCount(), 2)
    assert.equal(union.mode(), 'sparse')
    assert.equal(union.byteWidth(), null)

    const missing = optional.null()
    assert.equal(missing.isNull(), true)
    assert.equal(missing.value(), null)
    assert.equal(missing.scalar(), null)
    assert.equal(missing.asI64(), null)

    // The union reached through the data type is the same shape.
    assert.equal(new ty().optional().arrowFormat(), union.arrowFormat())
  })
}

test('out-of-range constructions throw actionable errors', () => {
  assert.throws(() => new data.Int8Scalar(1000), /int8/)
  assert.throws(() => new data.UInt8Scalar(-1), /uint8/)
  assert.throws(() => new data.UInt64Scalar(-1n), /uint64/)
  assert.throws(() => new data.Int64Scalar(2n ** 63n), /int64/)
})

test('float access is exact or null', () => {
  // 2n**53n is the last contiguous integer in f64; +1n rounds.
  assert.equal(new data.Int64Scalar(2n ** 53n).asF64(), 2 ** 53)
  assert.equal(new data.Int64Scalar(2n ** 53n + 1n).asF64(), null)
  assert.equal(new data.UInt64Scalar(2n ** 64n - 1n).asF64(), null)
  // Sign changes never pass.
  assert.equal(new data.Int8Scalar(-1).asU64(), null)
})

test('union field', () => {
  const union = new data.Int64().optional()
  const field = new data.UnionField('value', union)
  assert.equal(field.name(), 'value')
  assert.equal(field.isNullable(), true)
  assert.equal(field.dataType().arrowFormat(), '+us:0,1')
})

test('null family', () => {
  const nullType = new data.Null()
  assert.equal(nullType.name(), 'null')
  assert.equal(nullType.arrowFormat(), 'n')
  assert.equal(nullType.byteWidth(), null)
  assert.equal(nullType.bitWidth(), null)

  const gap = new data.NullField('gap')
  assert.deepEqual([gap.name(), gap.dataType().name(), gap.isNullable()], ['gap', 'null', true])

  const nothing = new data.NullScalar()
  assert.equal(nothing.isNull(), true)
  assert.equal(nothing.dataType().name(), 'null')
})
