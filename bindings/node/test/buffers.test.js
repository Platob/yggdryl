'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('../index.js')
const { ByteBuffer, BitBuffer, Whence } = yggdryl.core

test('ByteBuffer round-trips bytes', () => {
  const buf = new ByteBuffer()
  buf.pwriteByteArray(0, Whence.Start, Buffer.from([1, 2, 3]))
  assert.equal(buf.byteSize(), 3)
  assert.equal(buf.bitSize(), 24)
  assert.equal(buf.preadByteOne(1, Whence.Start), 2)
  assert.deepEqual([...buf.toBytes()], [1, 2, 3])
})

test('ByteBuffer bit access is MSB-first', () => {
  const buf = ByteBuffer.fromBytes(Buffer.from([0b1010_0000]))
  assert.equal(buf.preadBitOne(0, Whence.Start), true)
  assert.equal(buf.preadBitOne(1, Whence.Start), false)
})

test('ByteBuffer seek tracks the cursor', () => {
  const buf = ByteBuffer.fromBytes(Buffer.from([10, 20, 30, 40]))
  assert.equal(buf.seek(2, Whence.Start), 2)
  assert.equal(buf.tell(), 2)
  assert.equal(buf.preadByteOne(1, Whence.Current), 40)
})

test('BitBuffer tracks an exact bit length', () => {
  const buf = new BitBuffer()
  buf.pwriteBitArray(0, Whence.Start, [true, false, true])
  assert.equal(buf.bitSize(), 3)
  assert.equal(buf.byteSize(), 1)
  assert.deepEqual(buf.preadBitArray(0, Whence.Start, 3), [true, false, true])
})

test('capacity and resize', () => {
  const buf = ByteBuffer.fromBytes(Buffer.from([1, 2, 3]))
  assert.ok(buf.byteCapacity() >= 3)
  assert.ok(buf.resizeByteCapacity(64) >= 64)
  assert.equal(buf.byteSize(), 3) // capacity never changes the size
  assert.ok(buf.bitCapacity() >= 64 * 8)

  buf.resizeBytes(5)
  assert.deepEqual([...buf.toBytes()], [1, 2, 3, 0, 0])
  buf.resizeBytes(1)
  assert.deepEqual([...buf.toBytes()], [1])

  const bits = new BitBuffer()
  bits.resizeBits(3) // exact bit resize
  assert.equal(bits.bitSize(), 3)
  assert.equal(bits.byteSize(), 1)
})

test('stream copy between buffers', () => {
  const source = ByteBuffer.fromBytes(Buffer.from([1, 2, 3, 4]))
  const sink = new ByteBuffer()
  source.preadIo(1, Whence.Start, 3, sink, 0, Whence.Start)
  assert.deepEqual([...sink.toBytes()], [2, 3, 4])

  const appended = ByteBuffer.fromBytes(Buffer.from([9]))
  appended.pwriteIo(0, Whence.End, source, 0, Whence.Start, 2)
  assert.deepEqual([...appended.toBytes()], [9, 1, 2])
})
