'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { core, scalar } = yggdryl

// The 8-32 bit types carry values as `number`; the 64-bit types as `BigInt`.
const INTEGERS = [
  { scalarClass: scalar.Int8Scalar, optional: scalar.OptionalInt8Scalar, name: 'int8', low: -(2 ** 7), high: 2 ** 7 - 1, wire: (v) => v },
  { scalarClass: scalar.Int16Scalar, optional: scalar.OptionalInt16Scalar, name: 'int16', low: -(2 ** 15), high: 2 ** 15 - 1, wire: (v) => v },
  { scalarClass: scalar.Int32Scalar, optional: scalar.OptionalInt32Scalar, name: 'int32', low: -(2 ** 31), high: 2 ** 31 - 1, wire: (v) => v },
  { scalarClass: scalar.Int64Scalar, optional: scalar.OptionalInt64Scalar, name: 'int64', low: -(2n ** 63n), high: 2n ** 63n - 1n, wire: (v) => BigInt(v) },
  { scalarClass: scalar.UInt8Scalar, optional: scalar.OptionalUInt8Scalar, name: 'uint8', low: 0, high: 2 ** 8 - 1, wire: (v) => v },
  { scalarClass: scalar.UInt16Scalar, optional: scalar.OptionalUInt16Scalar, name: 'uint16', low: 0, high: 2 ** 16 - 1, wire: (v) => v },
  { scalarClass: scalar.UInt32Scalar, optional: scalar.OptionalUInt32Scalar, name: 'uint32', low: 0, high: 2 ** 32 - 1, wire: (v) => v },
  { scalarClass: scalar.UInt64Scalar, optional: scalar.OptionalUInt64Scalar, name: 'uint64', low: 0n, high: 2n ** 64n - 1n, wire: (v) => BigInt(v) },
]

for (const { scalarClass, optional, name, low, high, wire } of INTEGERS) {
  test(`${name} scalar holds a value or null`, () => {
    const answer = new scalarClass(wire(42))
    assert.equal(answer.isNull(), false)
    assert.equal(answer.value(), wire(42))
    assert.equal(answer.dataType().name(), name)
    assert.equal(new scalarClass(low).value(), low)
    assert.equal(new scalarClass(high).value(), high)

    const missing = scalarClass.null()
    assert.equal(missing.isNull(), true)
    assert.equal(missing.value(), null)
  })

  test(`${name} accessors convert exactly`, () => {
    const answer = new scalarClass(wire(42))
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
    // An integer is never a bool, a str or bytes: an actionable error.
    assert.throws(() => answer.asBool(), /no bool conversion/)
    assert.throws(() => answer.asStr(), /no str conversion/)
    assert.throws(() => answer.asBytes(), /no bytes conversion/)
    // A null scalar holds no value: every accessor throws.
    assert.throws(() => scalarClass.null().asI64(), /is null/)
  })

  test(`${name} optional scalar redirects to the inner scalar`, () => {
    const answer = new optional(wire(42))
    assert.equal(answer.isNull(), false)
    assert.equal(answer.value(), wire(42))
    assert.equal(answer.scalar().value(), wire(42))
    assert.equal(answer.asI64(), 42n)

    // The data type is the logical optional over union storage.
    const optType = answer.dataType()
    assert.equal(optType.name(), 'optional')
    assert.equal(optType.arrowFormat(), '+us:0,1')
    assert.equal(optType.byteWidth(), null)
    assert.equal(optType.valueType().name(), name)

    const missing = optional.null()
    assert.equal(missing.isNull(), true)
    assert.equal(missing.value(), null)
    assert.equal(missing.scalar(), null)
    assert.throws(() => missing.asI64(), /is null/)
  })
}

test('out-of-range constructions throw actionable errors', () => {
  assert.throws(() => new scalar.Int8Scalar(1000), /int8/)
  assert.throws(() => new scalar.UInt8Scalar(-1), /uint8/)
  assert.throws(() => new scalar.UInt64Scalar(-1n), /uint64/)
  assert.throws(() => new scalar.Int64Scalar(2n ** 63n), /int64/)
})

test('float access is exact or throws', () => {
  // 2n**53n is the last contiguous integer in f64; +1n rounds.
  assert.equal(new scalar.Int64Scalar(2n ** 53n).asF64(), 2 ** 53)
  assert.throws(() => new scalar.Int64Scalar(2n ** 53n + 1n).asF64(), /not exactly representable/)
  assert.throws(() => new scalar.UInt64Scalar(2n ** 64n - 1n).asF64(), /not exactly representable/)
  // Sign changes never pass, and the error names the offending value.
  assert.throws(() => new scalar.Int8Scalar(-1).asU64(), /-1 is not exactly representable/)
})

test('binary scalar reads bytes and io', () => {
  const blob = new scalar.BinaryScalar(Buffer.from([1, 2, 3]))
  assert.equal(blob.isNull(), false)
  assert.deepEqual(blob.value(), Buffer.from([1, 2, 3]))
  assert.deepEqual(blob.asBytes(), Buffer.from([1, 2, 3]))
  assert.equal(blob.dataType().name(), 'binary')
  // UTF-8 bytes convert to a string; anything else throws naming the shape —
  // and an explicit core charset decodes instead.
  assert.equal(new scalar.BinaryScalar(Buffer.from('hi')).asStr(), 'hi')
  assert.equal(new scalar.BinaryScalar(Buffer.from('hi')).asStr('utf8'), 'hi')
  assert.equal(new scalar.BinaryScalar(Buffer.from([0xe9])).asStr('latin1'), 'é')
  assert.throws(() => new scalar.BinaryScalar(Buffer.from([0xff])).asStr(), /non-UTF-8/)
  assert.throws(() => new scalar.BinaryScalar(Buffer.from('hi')).asStr('ascii'), /unknown charset/)
  assert.throws(() => blob.asI64(), /no i64 conversion/)

  // The value doubles as a core positioned-IO ByteBuffer.
  const io = blob.toIo()
  assert.equal(io.byteSize(), 3)
  assert.deepEqual(io.toBytes(), Buffer.from([1, 2, 3]))
  assert.equal(io.preadByteOne(1, core.Whence.Start), 2)

  // ... or as a full-window ByteBufferSlice for window-relative reads.
  const window = blob.toIoSlice()
  assert.equal(window.byteSize(), 3)
  assert.equal(window.preadByteOne(1, core.Whence.Start), 2)
  assert.equal(window.preadI8(2, core.Whence.Start), 3)

  // The empty value and null are distinct states.
  assert.equal(new scalar.BinaryScalar(Buffer.alloc(0)).isNull(), false)
  const missing = scalar.BinaryScalar.null()
  assert.equal(missing.isNull(), true)
  assert.equal(missing.value(), null)
  assert.equal(missing.toIo(), null)
  assert.throws(() => missing.asBytes(), /is null/)
})

test('optional binary redirects to the inner scalar', () => {
  const some = new scalar.OptionalBinaryScalar(Buffer.from('hi'))
  assert.equal(some.isNull(), false)
  assert.deepEqual(some.value(), Buffer.from('hi'))
  assert.deepEqual(some.scalar().value(), Buffer.from('hi'))
  assert.deepEqual(some.asBytes(), Buffer.from('hi'))
  assert.equal(some.asStr(), 'hi')

  const optType = some.dataType()
  assert.equal(optType.name(), 'optional')
  assert.equal(optType.valueType().name(), 'binary')
  assert.equal(optType.storage().name(), 'union')

  const missing = scalar.OptionalBinaryScalar.null()
  assert.equal(missing.isNull(), true)
  assert.equal(missing.scalar(), null)
  assert.throws(() => missing.asBytes(), /is null/)
})

test('null scalar', () => {
  const nothing = new scalar.NullScalar()
  assert.equal(nothing.isNull(), true)
  assert.equal(nothing.dataType().name(), 'null')
})

// The 8-32 bit series carry elements as `number`; the 64-bit series as `BigInt`.
const SERIES = [
  { serieClass: scalar.Int8Serie, name: 'int8', low: -(2 ** 7), high: 2 ** 7 - 1, wire: (v) => v },
  { serieClass: scalar.Int16Serie, name: 'int16', low: -(2 ** 15), high: 2 ** 15 - 1, wire: (v) => v },
  { serieClass: scalar.Int32Serie, name: 'int32', low: -(2 ** 31), high: 2 ** 31 - 1, wire: (v) => v },
  { serieClass: scalar.Int64Serie, name: 'int64', low: -(2n ** 63n), high: 2n ** 63n - 1n, wire: (v) => BigInt(v) },
  { serieClass: scalar.UInt8Serie, name: 'uint8', low: 0, high: 2 ** 8 - 1, wire: (v) => v },
  { serieClass: scalar.UInt16Serie, name: 'uint16', low: 0, high: 2 ** 16 - 1, wire: (v) => v },
  { serieClass: scalar.UInt32Serie, name: 'uint32', low: 0, high: 2 ** 32 - 1, wire: (v) => v },
  { serieClass: scalar.UInt64Serie, name: 'uint64', low: 0n, high: 2n ** 64n - 1n, wire: (v) => BigInt(v) },
]

for (const { serieClass, name, low, high, wire } of SERIES) {
  test(`${name} serie holds a sequence`, () => {
    const numbers = new serieClass([low, wire(2), high])
    assert.equal(numbers.isNull(), false)
    assert.equal(numbers.isEmpty(), false)
    assert.equal(numbers.len(), 3)
    assert.deepEqual(numbers.toArray(), [low, wire(2), high]) // extremes survive the buffer
    assert.equal(numbers.getAt(0), low)
    assert.equal(numbers.getAt(1), wire(2))
    assert.equal(numbers.getAt(2), high)
    assert.equal(numbers.getScalarAt(2).value(), high)
    assert.equal(numbers.getScalarAt(3), null) // out of bounds
    assert.equal(numbers.dataType().name(), 'list')
    assert.equal(numbers.dataType().valueType().name(), name)
    assert.throws(() => numbers.getAt(3)) // out of bounds
    assert.throws(() => numbers.getAt(-1), /non-negative index/) // negative, not wrapped
    assert.throws(() => numbers.getAt(2 ** 32)) // never aliased back into range
    assert.equal(numbers.getScalarAt(-1), null)
    assert.equal(numbers.getScalarAt(2 ** 32), null)

    // The empty serie and null are distinct states.
    const empty = new serieClass([])
    assert.equal(empty.isNull(), false)
    assert.equal(empty.isEmpty(), true)
    assert.deepEqual(empty.toArray(), [])

    const missing = serieClass.null()
    assert.equal(missing.isNull(), true)
    assert.equal(missing.toArray(), null)
    assert.throws(() => missing.getAt(0))
  })
}

test('toJsValue is the general native accessor', () => {
  // One FFI call per scalar: the class' wire type, or null when null.
  assert.equal(new scalar.Int32Scalar(42).toJsValue(), 42)
  assert.equal(new scalar.UInt8Scalar(255).toJsValue(), 255)
  assert.equal(new scalar.Int64Scalar(2n ** 63n - 1n).toJsValue(), 2n ** 63n - 1n)
  assert.equal(new scalar.UInt64Scalar(2n ** 64n - 1n).toJsValue(), 2n ** 64n - 1n)
  assert.equal(scalar.Int32Scalar.null().toJsValue(), null)
  assert.equal(scalar.Int64Scalar.null().toJsValue(), null)
  // Optionals mirror their inner scalar.
  assert.equal(new scalar.OptionalInt32Scalar(42).toJsValue(), 42)
  assert.equal(new scalar.OptionalInt64Scalar(42n).toJsValue(), 42n)
  assert.equal(scalar.OptionalInt64Scalar.null().toJsValue(), null)
  // Binary crosses as a Buffer.
  assert.deepEqual(new scalar.BinaryScalar(Buffer.from([1, 2])).toJsValue(), Buffer.from([1, 2]))
  assert.equal(scalar.BinaryScalar.null().toJsValue(), null)
  assert.equal(scalar.OptionalBinaryScalar.null().toJsValue(), null)
  // A serie crosses as the same array toArray() returns.
  assert.deepEqual(new scalar.Int32Serie([1, 2]).toJsValue(), [1, 2])
  assert.deepEqual(new scalar.Int64Serie([1n, 2n]).toJsValue(), [1n, 2n])
  assert.equal(scalar.Int64Serie.null().toJsValue(), null)
  // The null scalar is always null.
  assert.equal(new scalar.NullScalar().toJsValue(), null)
})

test('a serie element out of the value range is refused', () => {
  // The 8-32 bit constructors range-check each element with an actionable error.
  assert.throws(() => new scalar.Int8Serie([128]), /int8/)
  assert.throws(() => new scalar.UInt8Serie([-1]), /uint8/)
  // The 64-bit constructors refuse a BigInt outside the width.
  assert.throws(() => new scalar.Int64Serie([2n ** 63n]))
  assert.throws(() => new scalar.UInt64Serie([-1n]))
})
