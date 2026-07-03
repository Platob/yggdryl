'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { dtype } = yggdryl

// The 8-32 bit types carry codec values as `number`; the 64-bit types as `BigInt`.
const INTEGERS = [
  { ty: dtype.Int8, name: 'int8', format: 'c', width: 1, low: -(2 ** 7), high: 2 ** 7 - 1, wire: (v) => v },
  { ty: dtype.Int16, name: 'int16', format: 's', width: 2, low: -(2 ** 15), high: 2 ** 15 - 1, wire: (v) => v },
  { ty: dtype.Int32, name: 'int32', format: 'i', width: 4, low: -(2 ** 31), high: 2 ** 31 - 1, wire: (v) => v },
  { ty: dtype.Int64, name: 'int64', format: 'l', width: 8, low: -(2n ** 63n), high: 2n ** 63n - 1n, wire: (v) => BigInt(v) },
  { ty: dtype.UInt8, name: 'uint8', format: 'C', width: 1, low: 0, high: 2 ** 8 - 1, wire: (v) => v },
  { ty: dtype.UInt16, name: 'uint16', format: 'S', width: 2, low: 0, high: 2 ** 16 - 1, wire: (v) => v },
  { ty: dtype.UInt32, name: 'uint32', format: 'I', width: 4, low: 0, high: 2 ** 32 - 1, wire: (v) => v },
  { ty: dtype.UInt64, name: 'uint64', format: 'L', width: 8, low: 0n, high: 2n ** 64n - 1n, wire: (v) => BigInt(v) },
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
  const binary = new dtype.Binary()
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

test('optional binary type', () => {
  const optional = new dtype.Binary().optional()
  assert.equal(optional.name(), 'optional')
  assert.equal(optional.valueType().name(), 'binary')
  assert.equal(optional.storage().name(), 'union')
  assert.deepEqual(optional.defaultValue(), Buffer.alloc(0))
  assert.equal(optional.defaultScalar().isNull(), true)
  assert.deepEqual(
    optional.nativeFromBytes(optional.nativeToBytes(Buffer.from('xy'))),
    Buffer.from('xy'),
  )
  assert.equal(new dtype.OptionalBinary().arrowFormat(), optional.arrowFormat())
})

test('null type', () => {
  const nullType = new dtype.Null()
  assert.equal(nullType.name(), 'null')
  assert.equal(nullType.arrowFormat(), 'n')
  assert.equal(nullType.byteWidth(), null)
  assert.equal(nullType.bitWidth(), null)
})

test('union type reached through optional', () => {
  const union = new dtype.Int64().optional().storage()
  assert.equal(union.name(), 'union')
  assert.equal(union.arrowFormat(), '+us:0,1')
  assert.equal(union.byteWidth(), null)
  assert.equal(union.childCount(), 2)
  assert.equal(union.mode(), 'sparse')
})
