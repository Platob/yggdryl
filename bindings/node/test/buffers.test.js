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

test('ByteBuffer Current is measured from the start without a cursor', () => {
  const buf = ByteBuffer.fromBytes(Buffer.from([10, 20, 30, 40]))
  // A bare buffer keeps no cursor, so Current === Start.
  assert.equal(buf.preadByteOne(1, Whence.Current), 20)
  assert.equal(buf.preadByteOne(1, Whence.Start), 20)
})

test('out-of-bounds read throws', () => {
  const buf = ByteBuffer.fromBytes(Buffer.from([1, 2]))
  assert.throws(() => buf.preadByteArray(0, Whence.Start, 3), /out of bounds/)
})

test('BitBuffer tracks an exact bit length', () => {
  const buf = new BitBuffer()
  buf.pwriteBitArray(0, Whence.Start, [true, false, true])
  assert.equal(buf.bitSize(), 3)
  assert.equal(buf.byteSize(), 1)
  assert.deepEqual(buf.preadBitArray(0, Whence.Start, 3), [true, false, true])
})

test('ByteBuffer capacity and resize', () => {
  const buf = ByteBuffer.fromBytes(Buffer.from([1, 2, 3]))
  assert.ok(buf.byteCapacity() >= 3)
  assert.ok(buf.resizeByteCapacity(64) >= 64)
  assert.ok(buf.resizeBitCapacity(1024) >= 1024)
  assert.equal(buf.byteSize(), 3) // capacity never changes the size

  buf.resizeBytes(5)
  assert.deepEqual([...buf.toBytes()], [1, 2, 3, 0, 0])
  buf.resizeBytes(1)
  assert.deepEqual([...buf.toBytes()], [1])

  // ByteBuffer bit resizes round up to whole bytes.
  buf.resizeBits(9)
  assert.equal(buf.byteSize(), 2)
  assert.equal(buf.bitSize(), 16)
})

test('BitBuffer capacity and exact bit resize', () => {
  const buf = BitBuffer.fromBytes(Buffer.from([0xff, 0xff]))
  assert.ok(buf.byteCapacity() >= 2)
  assert.ok(buf.bitCapacity() >= 16)
  assert.ok(buf.resizeByteCapacity(32) >= 32)

  buf.resizeBytes(1) // sets bit_size to 8
  assert.equal(buf.bitSize(), 8)

  buf.resizeBits(3) // exact — and truncation zeroes padding
  assert.equal(buf.bitSize(), 3)
  assert.equal(buf.byteSize(), 1)
  assert.deepEqual([...buf.toBytes()], [0b1110_0000])
})
