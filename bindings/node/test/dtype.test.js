'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { dtype } = yggdryl

// The 8-32 bit types carry codec values as `number`; the 64-bit types as `BigInt`.
const INTEGERS = [
  { ty: dtype.Int8Type, name: 'int8', format: 'c', width: 1, low: -(2 ** 7), high: 2 ** 7 - 1, wire: (v) => v },
  { ty: dtype.Int16Type, name: 'int16', format: 's', width: 2, low: -(2 ** 15), high: 2 ** 15 - 1, wire: (v) => v },
  { ty: dtype.Int32Type, name: 'int32', format: 'i', width: 4, low: -(2 ** 31), high: 2 ** 31 - 1, wire: (v) => v },
  { ty: dtype.Int64Type, name: 'int64', format: 'l', width: 8, low: -(2n ** 63n), high: 2n ** 63n - 1n, wire: (v) => BigInt(v) },
  { ty: dtype.UInt8Type, name: 'uint8', format: 'C', width: 1, low: 0, high: 2 ** 8 - 1, wire: (v) => v },
  { ty: dtype.UInt16Type, name: 'uint16', format: 'S', width: 2, low: 0, high: 2 ** 16 - 1, wire: (v) => v },
  { ty: dtype.UInt32Type, name: 'uint32', format: 'I', width: 4, low: 0, high: 2 ** 32 - 1, wire: (v) => v },
  { ty: dtype.UInt64Type, name: 'uint64', format: 'L', width: 8, low: 0n, high: 2n ** 64n - 1n, wire: (v) => BigInt(v) },
]

for (const { ty, name, format, width, low, high, wire } of INTEGERS) {
  test(`${name} data type describes itself`, () => {
    const instance = new ty()
    assert.equal(instance.name(), name)
    assert.equal(instance.arrowFormat(), format)
    assert.equal(instance.byteWidth(), width)
    assert.equal(instance.bitWidth(), width * 8)
  })

  test(`${name} defaults`, () => {
    const instance = new ty()
    assert.equal(instance.defaultValue(), wire(0))
    assert.equal(instance.defaultScalar().value(), wire(0))

    const optional = instance.optional()
    assert.equal(optional.defaultValue(), wire(0))
    assert.equal(optional.defaultScalar().isNull(), true) // the null variant
  })

  test(`${name} data type is a factory for its field and scalar`, () => {
    const instance = new ty()

    // The data type builds its field: a name paired with the type.
    const column = instance.field('id', false)
    assert.equal(column.name(), 'id')
    assert.equal(column.dataType().name(), name)
    assert.equal(column.isNullable(), false)
    assert.equal(instance.field('maybe').isNullable(), true) // nullable by default

    // ... and its scalar, holding the native value.
    const answer = instance.scalar(wire(42))
    assert.equal(answer.isNull(), false)
    assert.equal(answer.value(), wire(42))
    assert.equal(answer.dataType().name(), name)

    // The optional data type is a factory too.
    const optional = instance.optional()
    const score = optional.field('score')
    assert.equal(score.name(), 'score')
    assert.equal(score.dataType().name(), 'optional')
    assert.equal(score.dataType().valueType().name(), name)
    const some = optional.scalar(wire(42))
    assert.equal(some.isNull(), false)
    assert.equal(some.value(), wire(42))
    assert.equal(some.dataType().valueType().name(), name)
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

  test(`${name} optional is a logical type over union storage`, () => {
    const optional = new ty().optional()
    assert.equal(optional.name(), 'optional')
    assert.equal(optional.arrowFormat(), '+us:0,1') // sparse, type ids 0 and 1
    assert.equal(optional.byteWidth(), null)
    assert.equal(optional.valueType().name(), name)

    const storage = optional.storage()
    assert.equal(storage.name(), 'union')
    assert.equal(storage.childCount(), 2)
    assert.equal(storage.mode(), 'sparse')

    // The optional's codec is the value type's.
    assert.equal(optional.nativeFromBytes(optional.nativeToBytes(wire(42))), wire(42))
  })
}

test('binary type describes itself and codecs', () => {
  const binary = new dtype.BinaryType()
  assert.equal(binary.name(), 'binary')
  assert.equal(binary.arrowFormat(), 'z')
  assert.equal(binary.byteWidth(), null)
  assert.equal(binary.bitWidth(), null)
  // The codec is the identity: any bytes are a valid binary value.
  assert.deepEqual(binary.nativeToBytes(Buffer.from([1, 2])), Buffer.from([1, 2]))
  assert.deepEqual(binary.nativeFromBytes(Buffer.from([1, 2])), Buffer.from([1, 2]))
  assert.deepEqual(binary.nativeFromBytes(Buffer.alloc(0)), Buffer.alloc(0))
  assert.deepEqual(binary.defaultValue(), Buffer.alloc(0))
  assert.deepEqual(binary.defaultScalar().value(), Buffer.alloc(0))
})

test('binary type is a factory for its field and scalar', () => {
  const binary = new dtype.BinaryType()
  const payload = binary.field('payload', false)
  assert.equal(payload.name(), 'payload')
  assert.equal(payload.dataType().name(), 'binary')
  assert.equal(payload.isNullable(), false)
  assert.equal(binary.field('maybe').isNullable(), true)
  assert.deepEqual(binary.scalar(Buffer.from('xy')).value(), Buffer.from('xy'))
})

test('optional binary type', () => {
  const optional = new dtype.BinaryType().optional()
  assert.equal(optional.name(), 'optional')
  assert.equal(optional.valueType().name(), 'binary')
  assert.equal(optional.storage().name(), 'union')
  assert.deepEqual(optional.defaultValue(), Buffer.alloc(0))
  assert.equal(optional.defaultScalar().isNull(), true)
  assert.deepEqual(
    optional.nativeFromBytes(optional.nativeToBytes(Buffer.from('xy'))),
    Buffer.from('xy'),
  )
  assert.equal(new dtype.OptionalBinaryType().arrowFormat(), optional.arrowFormat())
  // The optional binary type is a factory too.
  assert.equal(optional.field('payload').dataType().name(), 'optional')
  assert.deepEqual(optional.scalar(Buffer.from('xy')).value(), Buffer.from('xy'))
})

test('null type', () => {
  const nullType = new dtype.NullType()
  assert.equal(nullType.name(), 'null')
  assert.equal(nullType.arrowFormat(), 'n')
  assert.equal(nullType.byteWidth(), null)
  assert.equal(nullType.bitWidth(), null)
})

test('union type reached through optional', () => {
  const union = new dtype.Int64Type().optional().storage()
  assert.equal(union.name(), 'union')
  assert.equal(union.arrowFormat(), '+us:0,1')
  assert.equal(union.byteWidth(), null)
  assert.equal(union.childCount(), 2)
  assert.equal(union.mode(), 'sparse')
})

// The 8-32 bit series carry elements as `number`; the 64-bit series as `BigInt`.
const SERIES = [
  { ty: dtype.Int8SerieType, name: 'int8', width: 1, low: -(2 ** 7), high: 2 ** 7 - 1, wire: (v) => v },
  { ty: dtype.Int16SerieType, name: 'int16', width: 2, low: -(2 ** 15), high: 2 ** 15 - 1, wire: (v) => v },
  { ty: dtype.Int32SerieType, name: 'int32', width: 4, low: -(2 ** 31), high: 2 ** 31 - 1, wire: (v) => v },
  { ty: dtype.Int64SerieType, name: 'int64', width: 8, low: -(2n ** 63n), high: 2n ** 63n - 1n, wire: (v) => BigInt(v) },
  { ty: dtype.UInt8SerieType, name: 'uint8', width: 1, low: 0, high: 2 ** 8 - 1, wire: (v) => v },
  { ty: dtype.UInt16SerieType, name: 'uint16', width: 2, low: 0, high: 2 ** 16 - 1, wire: (v) => v },
  { ty: dtype.UInt32SerieType, name: 'uint32', width: 4, low: 0, high: 2 ** 32 - 1, wire: (v) => v },
  { ty: dtype.UInt64SerieType, name: 'uint64', width: 8, low: 0n, high: 2n ** 64n - 1n, wire: (v) => BigInt(v) },
]

for (const { ty, name, width, low, high, wire } of SERIES) {
  test(`${name} serie type describes itself`, () => {
    const serie = new ty()
    assert.equal(serie.name(), 'list')
    assert.equal(serie.arrowFormat(), '+l')
    assert.equal(serie.byteWidth(), null)
    assert.equal(serie.bitWidth(), null)
    assert.equal(serie.childCount(), 1)
    assert.equal(serie.valueType().name(), name)
  })

  test(`${name} serie codec round-trips`, () => {
    const serie = new ty()
    // The codec concatenates the value type's per-element bytes, extremes included.
    const encoded = serie.nativeToBytes([low, wire(0), high])
    assert.equal(encoded.length, 3 * width)
    assert.deepEqual(serie.nativeFromBytes(encoded), [low, wire(0), high])
    assert.deepEqual(serie.nativeFromBytes(Buffer.alloc(0)), [])
    if (width > 1) { // every length is whole for the 1-byte widths
      assert.throws(() => serie.nativeFromBytes(Buffer.alloc(width + 1)))
    }
  })

  test(`${name} serie type is a factory`, () => {
    const serie = new ty()
    assert.deepEqual(serie.defaultValue(), [])
    assert.equal(serie.defaultScalar().isNull(), false)
    assert.equal(serie.defaultScalar().len(), 0)

    const column = serie.field('scores')
    assert.equal(column.name(), 'scores')
    assert.equal(column.dataType().name(), 'list')
    assert.equal(column.isNullable(), true)
    assert.equal(serie.field('scores', false).isNullable(), false)

    const numbers = serie.scalar([low, wire(0), high])
    assert.deepEqual(numbers.toArray(), [low, wire(0), high])
    assert.equal(numbers.dataType().valueType().name(), name)
  })
}

test('data types render a compact signature', () => {
  assert.equal(new dtype.Int64Type().display(), 'int64')
  assert.equal(String(new dtype.Int64Type()), 'int64') // napi maps display() to toString()
  assert.equal(`${new dtype.Float64Type()}`, 'float64') // template literals too
  assert.equal(new dtype.OptionalInt64Type().display(), 'optional<int64>')
  assert.equal(new dtype.Int64SerieType().display(), 'list<int64>')
  assert.equal(new dtype.Utf8Type().display(), 'utf8')
})
