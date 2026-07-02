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
