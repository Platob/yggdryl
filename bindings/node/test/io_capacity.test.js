'use strict'

// ByteCursor auto-resize, capacity reduction, and negative-seek edge cases.
// Mirrors the core tests/io_edge_cases.rs.

const test = require('node:test')
const assert = require('node:assert/strict')

const { ByteBuffer, Whence } = require('..').io

test('write past end auto-grows', () => {
  const cursor = new ByteBuffer(Buffer.from('ab')).byteCursor()
  cursor.seek(5, Whence.Start) // past the 2-byte end
  cursor.pwriteByteArray(Buffer.from('XY'), Whence.Current)
  assert.deepEqual(cursor.asBytes(), Buffer.from([0x61, 0x62, 0, 0, 0, 0x58, 0x59]))
})

test('append grows capacity', () => {
  const cursor = ByteBuffer.withByteCapacity(2).byteCursor()
  cursor.pwriteByteArray(Buffer.alloc(1000), Whence.Start)
  assert.ok(Number(cursor.byteCapacity()) >= 1000)
})

test('set byte capacity reserves above', () => {
  const cursor = new ByteBuffer(Buffer.from('abc')).byteCursor()
  assert.ok(Number(cursor.setByteCapacity(128)) >= 128)
  assert.deepEqual(cursor.asBytes(), Buffer.from('abc')) // content untouched
})

test('set byte capacity reduces below length', () => {
  const cursor = new ByteBuffer(Buffer.from('abcdefgh')).byteCursor()
  cursor.seek(0, Whence.End)
  cursor.setByteCapacity(3) // below length -> reduce the inner buffer
  assert.deepEqual(cursor.asBytes(), Buffer.from('abc'))
  assert.equal(Number(cursor.position()), 3) // clamped
})

test('set byte capacity leaves source intact', () => {
  const buf = new ByteBuffer(Buffer.from('shared'))
  const cursor = buf.byteCursor()
  cursor.setByteCapacity(2)
  assert.deepEqual(cursor.asBytes(), Buffer.from('sh'))
  assert.deepEqual(buf.asBytes(), Buffer.from('shared'))
})

test('negative seek before start throws', () => {
  const cursor = new ByteBuffer(Buffer.from('abc')).byteCursor()
  assert.throws(() => cursor.seek(-1, Whence.Start), /before the start/)
})

test('negative seek from end resolves', () => {
  const cursor = new ByteBuffer(Buffer.from('0123456789')).byteCursor()
  assert.equal(Number(cursor.seek(-3, Whence.End)), 7)
})
