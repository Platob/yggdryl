'use strict'

// ByteCursor.write — runtime type inference (Buffer / string / bigint[] / number[]).

const test = require('node:test')
const assert = require('node:assert/strict')

const { ByteBuffer, Whence } = require('..').io

test('write Buffer', () => {
  const c = new ByteBuffer().byteCursor()
  assert.equal(Number(c.write(Buffer.from('hello'))), 5)
  assert.deepEqual(c.asBytes(), Buffer.from('hello'))
})

test('write string as utf-8', () => {
  const c = new ByteBuffer().byteCursor()
  assert.equal(Number(c.write('café')), Buffer.from('café').length)
  assert.deepEqual(c.asBytes(), Buffer.from('café'))
})

test('write bigint array as i64', () => {
  const c = new ByteBuffer().byteCursor()
  assert.equal(Number(c.write([1n, 2n, 3n])), 24) // 3 * 8 bytes
  assert.deepEqual(
    c.preadI64Array(3, Whence.Start).map(Number),
    [1, 2, 3],
  )
})

test('write number array as f64', () => {
  const c = new ByteBuffer().byteCursor()
  assert.equal(Number(c.write([1.5, 2.5])), 16)
  assert.deepEqual(c.preadF64Array(2, Whence.Start), [1.5, 2.5])
})

test('write empty array is zero', () => {
  const c = new ByteBuffer().byteCursor()
  assert.equal(Number(c.write([])), 0)
})

test('write at whence auto-grows', () => {
  const c = new ByteBuffer(Buffer.from('ab')).byteCursor()
  c.seek(5, Whence.Start)
  c.write(Buffer.from('XY'), Whence.Current)
  assert.equal(c.asBytes().length, 7)
})

test('write mixed array rejected', () => {
  const c = new ByteBuffer().byteCursor()
  assert.throws(() => c.write([1n, 2.5]), /mixed/)
})

test('write boolean array points at BooleanBuffer (matches Python)', () => {
  const c = new ByteBuffer().byteCursor()
  assert.throws(() => c.write([true, false]), /BooleanBuffer/)
})

test('write rejects out-of-range bigint (no silent truncation)', () => {
  const c = new ByteBuffer().byteCursor()
  assert.throws(() => c.write([2n ** 64n]), /signed 64-bit range/)
})
