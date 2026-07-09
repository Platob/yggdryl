'use strict'

// Parity fixes: Node surfaces that lagged Python (found in the global coherence
// review) — f32 array methods on ByteCursor, bit/byteCapacity methods on typed
// slices, the full F32Slice surface, and ByteSlice.preadInto.

const test = require('node:test')
const assert = require('node:assert/strict')

const { ByteBuffer, F32Slice, I32Slice, Whence } = require('..').io

test('ByteCursor f32 array round-trip', () => {
  const c = new ByteBuffer().byteCursor()
  c.pwriteF32Array([1.5, 2.5, 3.5], Whence.Start)
  assert.deepEqual(c.preadF32Array(3, Whence.Start), [1.5, 2.5, 3.5])
})

test('typed slice exposes bit + byteCapacity (parity with Python)', () => {
  const data = Buffer.from(Int32Array.from([1, 2, 3, 4]).buffer)
  const sl = I32Slice.fromBytes(data, 0, data.length)
  assert.equal(Number(sl.bitTell()), 0)
  sl.bitSeek(0, Whence.Start)
  assert.ok(Number(sl.bitSize()) >= 0)
  assert.ok(Number(sl.byteCapacity()) >= 0)
})

test('F32Slice has the full typed-slice surface', () => {
  const data = Buffer.from(Float32Array.from([1.5, 2.5]).buffer)
  const sl = F32Slice.fromBytes(data, 0, data.length)
  assert.equal(Number(sl.byteTell()), 0)
  sl.byteSeek(0, Whence.Start)
  sl.bitSeek(0, Whence.Start)
  assert.equal(Number(sl.position()), 0)
  sl.setPosition(0)
  assert.ok(Number(sl.capacity()) >= 0)
  assert.ok(Number(sl.byteSize()) >= 0)
  assert.ok(Number(sl.bitSize()) >= 0)
  assert.ok(Number(sl.byteCapacity()) >= 0)
  assert.deepEqual(sl.preadArray(2, Whence.Start), [1.5, 2.5])
})

test('ByteSlice preadInto fills a Buffer in place', () => {
  const buf = new ByteBuffer(Buffer.from('abcdef'))
  const sl = buf.byteSlice(0, 6)
  const dst = Buffer.alloc(3)
  assert.equal(Number(sl.preadInto(dst, Whence.Start)), 3)
  assert.deepEqual(dst, Buffer.from('abc'))
})
