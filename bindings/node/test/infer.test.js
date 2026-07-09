'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')

const { buffer } = yggdryl.infer
const { I64Buffer, F64Buffer, BooleanBuffer, U8Buffer } = yggdryl.buffer

test('infers bigint array as I64Buffer', () => {
  const buf = buffer([10n, 20n, 30n])
  assert.ok(buf instanceof I64Buffer)
  // The I64Buffer constructor takes JS numbers (napi maps an `i64` arg from a
  // `number`); inference keys on `bigint` to tell integers from floats.
  assert.ok(buf.equals(new I64Buffer([10, 20, 30])))
})

test('infers number array as F64Buffer', () => {
  const buf = buffer([1.5, 2.5])
  assert.ok(buf instanceof F64Buffer)
  assert.ok(buf.equals(new F64Buffer([1.5, 2.5])))
})

test('infers boolean array as BooleanBuffer', () => {
  const buf = buffer([true, false, true])
  assert.ok(buf instanceof BooleanBuffer)
  assert.ok(buf.equals(new BooleanBuffer([true, false, true])))
})

test('infers a Buffer as U8Buffer', () => {
  const buf = buffer(Buffer.from([1, 2, 3]))
  assert.ok(buf instanceof U8Buffer)
  assert.ok(buf.equals(new U8Buffer([1, 2, 3])))
})

test('empty array is a guided error', () => {
  assert.throws(() => buffer([]), /empty array/)
})

test('out-of-i64-range bigint names the remedy', () => {
  assert.throws(() => buffer([2n ** 64n]), /signed 64-bit range/)
})

test('mixed array is rejected', () => {
  assert.throws(() => buffer([1n, 2.5]), /every element must be a bigint/)
})

test('unsupported element type is a guided error', () => {
  assert.throws(() => buffer(['a', 'b']), /boolean, bigint, and number/)
})
