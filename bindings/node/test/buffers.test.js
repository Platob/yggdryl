'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('../index.js')
const { ByteBuffer, BitBuffer, Whence, ByteBufferCursor, ByteBufferSlice } = yggdryl.core

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

test('ByteBuffer.cursor advances on reads over a copy', () => {
  const buf = ByteBuffer.fromBytes(Buffer.from([10, 20, 30, 40]))
  const cursor = buf.cursor()
  assert.deepEqual([...cursor.preadByteArray(0, Whence.Current, 2)], [10, 20])
  assert.equal(cursor.tell(), 2)
  assert.deepEqual([...cursor.preadByteArray(0, Whence.Current, 2)], [30, 40])
  assert.equal(cursor.tell(), 4)

  // The cursor holds a copy: writing through it leaves the original buffer intact.
  cursor.seek(0, Whence.Start)
  cursor.pwriteByteOne(0, Whence.Current, 99)
  assert.deepEqual([...cursor.toBytes()], [99, 20, 30, 40])
  assert.deepEqual([...buf.toBytes()], [10, 20, 30, 40])
})

test('ByteBufferCursor.fromBytes constructs directly', () => {
  const cursor = ByteBufferCursor.fromBytes(Buffer.from([1, 2, 3]))
  assert.equal(cursor.byteSize(), 3)
  assert.equal(cursor.seek(1, Whence.Start), 1)
  assert.equal(cursor.preadByteOne(0, Whence.Current), 2)
})

test('ByteBuffer.slice bounds access to a window', () => {
  const buf = ByteBuffer.fromBytes(Buffer.from([10, 20, 30, 40, 50]))
  const slice = buf.slice(1, 4)
  assert.equal(slice.byteSize(), 3)
  assert.equal(slice.start(), 1)
  assert.equal(slice.end(), 4)
  assert.deepEqual([...slice.preadByteArray(0, Whence.Start, 3)], [20, 30, 40])
  // Access outside the window throws.
  assert.throws(() => slice.preadByteArray(0, Whence.Start, 4), /out of bounds/)
  // Writes stay in-window and reach the slice's copy of the buffer.
  slice.pwriteByteOne(0, Whence.Start, 99)
  assert.deepEqual([...slice.toBytes()], [10, 99, 30, 40, 50])
})

test('ByteBufferSlice.fromBytes constructs directly', () => {
  const slice = ByteBufferSlice.fromBytes(Buffer.from([1, 2, 3, 4]), 1, 3)
  assert.equal(slice.byteSize(), 2)
  assert.deepEqual([...slice.preadByteArray(0, Whence.Start, 2)], [2, 3])
})

test('BitBuffer exposes cursor and slice too', () => {
  const cursor = BitBuffer.fromBytes(Buffer.from([0xff])).cursor()
  assert.equal(cursor.bitSize(), 8)
  assert.equal(cursor.preadBitOne(0, Whence.Current), true)

  const slice = BitBuffer.fromBytes(Buffer.from([1, 2, 3])).slice(1, 2)
  assert.equal(slice.byteSize(), 1)
  assert.equal(slice.preadByteOne(0, Whence.Start), 2) // window byte 1 of the inner
})
