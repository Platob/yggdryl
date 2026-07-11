'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const { converter } = require('..')

test('cast widens bytes', () => {
  const data = Buffer.alloc(4)
  data.writeInt32LE(7)
  const wide = converter.cast(data, 'i32', 'i64')
  const expected = Buffer.alloc(8)
  expected.writeBigInt64LE(7n)
  assert.ok(wide.equals(expected))
})

test('cast narrows bytes', () => {
  const data = Buffer.alloc(4)
  data.writeInt32LE(258)
  assert.ok(converter.cast(data, 'i32', 'u8').equals(Buffer.from([2])))
})

test('cast unknown dtype is guided', () => {
  assert.throws(() => converter.cast(Buffer.alloc(4), 'i32', 'i128'), /i8, i16/)
})

test('parse flexible formats yield a number', () => {
  assert.equal(converter.parse('42', 'i32'), 42)
  assert.equal(converter.parse('0x2A', 'i32'), 42)
  assert.equal(converter.parse('0b101010', 'u8'), 42)
  assert.equal(converter.parse('  -7 ', 'i16'), -7)
  assert.equal(converter.parse('1.5e3', 'f64'), 1500)
})

test('parse of i64 / u64 yields a bigint', () => {
  assert.equal(converter.parse('-1_000', 'i64'), -1000n)
  assert.equal(typeof converter.parse('5', 'i64'), 'bigint')
})

test('parse accepts comma separators', () => {
  assert.equal(converter.parse('1,000,000', 'i64'), 1000000n)
  assert.equal(converter.parse('1,234.5', 'f64'), 1234.5)
})

test('parse out of range shows the value', () => {
  assert.throws(() => converter.parse('99999999999', 'i32'), /out of range/)
})

test('convert numeric scalars', () => {
  assert.equal(converter.convert(300, 'i32', 'u8'), 44) // 300 & 0xFF
  assert.equal(converter.convert(3, 'i32', 'f32'), 3)
  assert.equal(converter.convert(-1n, 'i64', 'i64'), -1n)
})

test('convert rejects out-of-range / fractional input (matches Python strictness)', () => {
  assert.throws(() => converter.convert(300, 'i8', 'i16'), /range for i8/) // 300 > i8 max
  assert.throws(() => converter.convert(3.5, 'i8', 'i16'), /not an integer/) // fractional
  assert.throws(() => converter.format(3.5, 'i8'), /not an integer/)
  // 64-bit dtypes: a bigint past the range is rejected (core check), not truncated.
  assert.throws(() => converter.convert(2n ** 63n, 'i64', 'i64'), /out of range for i64/)
  assert.throws(() => converter.convert(-1n, 'u64', 'u64'), /out of range for u64/)
  // A bigint beyond 128 bits cannot even be represented — guided error, no wraparound.
  assert.throws(() => converter.convert(2n ** 200n, 'i64', 'i64'), /exceeds 128 bits/)
})

test('parse failure is guided', () => {
  assert.throws(() => converter.parse('twelve', 'i32'), /0x-hex/)
  assert.throws(() => converter.parse('-1', 'u8'))
})

test('format round trips', () => {
  assert.equal(converter.format(42, 'i32'), '42')
  assert.equal(converter.format(-7n, 'i64'), '-7')
  assert.equal(converter.parse(converter.format(-123, 'i16'), 'i16'), -123)
})

test('utf8 round trip and validation', () => {
  assert.ok(converter.utf8Encode('café').equals(Buffer.from('café', 'utf8')))
  assert.equal(converter.utf8Decode(Buffer.from('café', 'utf8')), 'café')
  assert.throws(() => converter.utf8Decode(Buffer.from([0xff])), /UTF-8/)
})

test('convertBytes cast round trips', () => {
  const data = Buffer.alloc(4)
  data.writeInt32LE(7)
  const wide = converter.convertBytes(data, 'cast', 'i32', 'i64')
  const expected = Buffer.alloc(8)
  expected.writeBigInt64LE(7n)
  assert.ok(wide.equals(expected))
  // invert casts the i64 bytes back to i32.
  assert.ok(converter.invertBytes(wide, 'cast', 'i32', 'i64').equals(data))
})

test('convertBytes string convert and invert', () => {
  // 'overall' string convert: UTF-8 text bytes -> i32 little-endian bytes.
  const le = converter.convertBytes(Buffer.from('42'), 'string', 'i32')
  const expected = Buffer.alloc(4)
  expected.writeInt32LE(42)
  assert.ok(le.equals(expected))
  // invert string: i32 bytes -> decimal text bytes.
  assert.ok(converter.invertBytes(le, 'string', 'i32').equals(Buffer.from('42')))
})

test('convertBytes bytes and utf8 kinds', () => {
  const payload = Buffer.alloc(4)
  payload.writeInt32LE(258)
  assert.ok(converter.convertBytes(payload, 'bytes', 'i32').equals(payload))
  assert.ok(converter.invertBytes(payload, 'bytes', 'i32').equals(payload))
  assert.ok(converter.convertBytes(Buffer.from('c'), 'utf8').equals(Buffer.from('c')))
  assert.throws(() => converter.convertBytes(Buffer.from([0xff]), 'utf8'), /UTF-8/)
})

test('convertBytes is guided', () => {
  assert.throws(() => converter.convertBytes(Buffer.alloc(0), 'nope', 'i32'), /unknown converter/)
  assert.throws(() => converter.convertBytes(Buffer.alloc(4), 'cast', 'i32'), /needs a to dtype/)
  assert.throws(() => converter.convertBytes(Buffer.from('42'), 'string'), /needs a dtype/)
})
