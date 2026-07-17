'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { Heap, Whence } = yggdryl.memory

// -------------------------------------------------------------------------------------
// Namespace + construction
// -------------------------------------------------------------------------------------

test('the memory namespace exposes Heap and Whence', () => {
  assert.equal(typeof Heap, 'function')
  assert.equal(Whence.Start, 0)
  assert.equal(Whence.Current, 1)
  assert.equal(Whence.End, 2)
})

test('construction: empty, from Buffer, withCapacity', () => {
  const empty = new Heap()
  assert.ok(empty.isEmpty())
  assert.equal(empty.byteSize(), 0)

  const fromBuf = new Heap(Buffer.from('abcd'))
  assert.equal(fromBuf.byteSize(), 4)
  assert.ok(!fromBuf.isEmpty())
  assert.deepEqual(fromBuf.toBytes(), Buffer.from('abcd'))

  const reserved = Heap.withCapacity(64)
  assert.ok(reserved.isEmpty())
  assert.ok(reserved.capacity() >= 64)
})

test('constructor copies the source buffer (later mutation is independent)', () => {
  const src = Buffer.from('abc')
  const h = new Heap(src)
  src[0] = 0x5a // 'Z'
  assert.deepEqual(h.toBytes(), Buffer.from('abc')) // heap holds its own copy
})

// -------------------------------------------------------------------------------------
// Size + capacity
// -------------------------------------------------------------------------------------

test('byteSize / bitSize / isEmpty', () => {
  const h = new Heap(Buffer.from('abcd'))
  assert.equal(h.byteSize(), 4)
  assert.equal(h.bitSize(), 32)
  assert.ok(!h.isEmpty())

  assert.ok(new Heap().isEmpty())
  assert.equal(new Heap().bitSize(), 0)
})

test('withCapacity + reserve grow capacity but not size', () => {
  const h = Heap.withCapacity(64)
  assert.ok(h.isEmpty())
  assert.ok(h.capacity() >= 64)

  // Writing within capacity keeps the same allocation.
  const cap = h.capacity()
  h.pwriteByteArray(0, Buffer.from([1, 2, 3, 4]))
  assert.equal(h.byteSize(), 4)
  assert.equal(h.capacity(), cap)

  // reserve grows capacity but not size.
  h.reserve(1000)
  assert.ok(h.capacity() >= 1004)
  assert.equal(h.byteSize(), 4)
})

// -------------------------------------------------------------------------------------
// Byte-array primitives
// -------------------------------------------------------------------------------------

test('preadByteArray reads short and empty near the end', () => {
  const h = new Heap(Buffer.from('hello'))
  assert.deepEqual(h.preadByteArray(2, 8), Buffer.from('llo')) // only 3 remain from offset 2
  assert.deepEqual(h.preadByteArray(5, 8), Buffer.alloc(0)) // at the end
  assert.deepEqual(h.preadByteArray(99, 8), Buffer.alloc(0)) // past the end
})

test('pwriteByteArray grows and zero-fills the gap', () => {
  const h = new Heap(Buffer.from('abc'))
  assert.equal(h.pwriteByteArray(5, Buffer.from('Z')), 1)
  assert.deepEqual(h.toBytes(), Buffer.from([0x61, 0x62, 0x63, 0, 0, 0x5a])) // "abc\0\0Z"

  // Overwriting in place does not grow.
  assert.equal(h.pwriteByteArray(0, Buffer.from('XY')), 2)
  assert.deepEqual(h.toBytes(), Buffer.from([0x58, 0x59, 0x63, 0, 0, 0x5a])) // "XYc\0\0Z"

  // Empty write is a no-op.
  assert.equal(h.pwriteByteArray(0, Buffer.alloc(0)), 0)
})

// -------------------------------------------------------------------------------------
// Typed positioned accessors: byte / bit / i32 / i64
// -------------------------------------------------------------------------------------

test('typed byte round-trip, grow + zero-fill, and EOF throws', () => {
  const h = new Heap()
  h.pwriteByte(3, 0xab) // grows to 4, zero-filling 0..3
  assert.deepEqual(h.toBytes(), Buffer.from([0, 0, 0, 0xab]))
  assert.equal(h.preadByte(3), 0xab)
  assert.equal(h.preadByte(0), 0)
  assert.throws(() => h.preadByte(4), /unexpected end of data/)
})

test('typed bit is LSB-first, grows on write, read past end throws', () => {
  const h = new Heap(Buffer.from([0b0000_0101, 0b1000_0000]))
  assert.equal(h.preadBit(0), true) // byte 0, bit 0
  assert.equal(h.preadBit(1), false)
  assert.equal(h.preadBit(2), true)
  assert.equal(h.preadBit(15), true) // byte 1, bit 7 (MSB)
  assert.equal(h.preadBit(8), false)
  assert.throws(() => h.preadBit(16), /unexpected end of data/)

  const w = new Heap()
  w.pwriteBit(10, true) // byte 1, bit 2 — grows to 2 bytes
  assert.deepEqual(w.toBytes(), Buffer.from([0b0000_0000, 0b0000_0100]))
  assert.equal(w.preadBit(10), true)
  // Clearing a set bit, read-modify-write.
  w.pwriteBit(10, false)
  assert.deepEqual(w.toBytes(), Buffer.from([0, 0]))
  // A second bit in the same byte preserves the first.
  w.pwriteBit(1, true)
  w.pwriteBit(3, true)
  assert.equal(w.toBytes()[0], 0b0000_1010)
})

test('typed i32 / i64 are little-endian round-trips; short data throws', () => {
  const h = new Heap()
  h.pwriteI32(0, -42)
  h.pwriteI64(4, 1234567890123)
  assert.deepEqual(h.preadByteArray(0, 4), Buffer.from(Int32Array.of(-42).buffer)) // LE bytes
  assert.equal(h.preadI32(0), -42)
  assert.equal(h.preadI64(4), 1234567890123)

  const small = new Heap(Buffer.from('abc'))
  assert.throws(() => small.preadI32(0), /unexpected end of data/) // needs 4, only 3
  assert.throws(() => small.preadI64(0), /unexpected end of data/)
})

// -------------------------------------------------------------------------------------
// Cursor stream
// -------------------------------------------------------------------------------------

test('cursor read/write advance the position', () => {
  const h = new Heap()
  assert.equal(h.write(Buffer.from('hello')), 5)
  assert.equal(h.position, 5)
  h.rewind()
  assert.equal(h.position, 0)
  assert.deepEqual(h.read(5), Buffer.from('hello'))
  assert.equal(h.position, 5)
  // Reading at the end yields an empty buffer, cursor unmoved.
  assert.deepEqual(h.read(5), Buffer.alloc(0))
  assert.equal(h.position, 5)
})

test('cursor typed round-trip and read-past-end throws leaving the cursor put', () => {
  const h = new Heap()
  h.writeByte(0x7f)
  h.writeI32(-7)
  h.writeI64(Math.pow(2, 40)) // 1 << 40, below 2^53
  assert.equal(h.position, 1 + 4 + 8)
  h.rewind()
  assert.equal(h.readByte(), 0x7f)
  assert.equal(h.readI32(), -7)
  assert.equal(h.readI64(), Math.pow(2, 40))

  const pos = h.position
  assert.throws(() => h.readByte(), /unexpected end of data/)
  assert.equal(h.position, pos, 'a failed read must not advance the cursor')
})

test('readToEnd reads from the cursor to the end', () => {
  const h = new Heap(Buffer.from('hello world'))
  h.seek(Whence.Start, 6)
  assert.deepEqual(h.readToEnd(), Buffer.from('world'))
  assert.equal(h.position, 11)
  assert.deepEqual(h.readToEnd(), Buffer.alloc(0)) // already at the end
})

test('setPosition moves the cursor; a write past the end zero-fills', () => {
  const h = new Heap()
  h.setPosition(4)
  assert.equal(h.position, 4)
  h.write(Buffer.from('Z'))
  assert.deepEqual(h.toBytes(), Buffer.from([0, 0, 0, 0, 0x5a]))
})

// -------------------------------------------------------------------------------------
// Seek / Whence
// -------------------------------------------------------------------------------------

test('seek from every Whence anchor; before-start throws', () => {
  const h = new Heap(Buffer.from('hello world'))
  assert.equal(h.seek(Whence.Start, 6), 6)
  assert.equal(h.seek(Whence.Current, -1), 5)
  assert.equal(h.seek(Whence.End, -5), 6)
  // Past the end is allowed.
  assert.equal(h.seek(Whence.End, 10), 21)
  // Before the start is not.
  assert.throws(() => h.seek(Whence.Start, -1), /invalid seek/)
  assert.throws(() => h.seek(Whence.Start, -1), /before the start/)
})

// -------------------------------------------------------------------------------------
// Slice
// -------------------------------------------------------------------------------------

test('slice yields an independent window addressed from its own 0', () => {
  const h = new Heap(Buffer.from('hello world'))
  const world = h.slice(6, 5)
  assert.ok(world instanceof Heap)
  assert.equal(world.byteSize(), 5)
  assert.deepEqual(world.toBytes(), Buffer.from('world'))
  // A window can be sliced again from its own 0.
  assert.deepEqual(world.slice(0, 2).toBytes(), Buffer.from('wo'))
  // The window is independent — mutating it leaves the parent untouched.
  world.pwriteByte(0, 0x5a)
  assert.deepEqual(h.toBytes(), Buffer.from('hello world'))
})

test('slice out of bounds throws a guided error', () => {
  const h = new Heap(Buffer.from('hello world'))
  assert.throws(() => h.slice(6, 6), /runs past the end/) // 6 + 6 > 11
  assert.throws(() => h.slice(6, 6), /11/)
})

// -------------------------------------------------------------------------------------
// Value semantics
// -------------------------------------------------------------------------------------

test('toBytes copies the stored bytes (excludes the cursor)', () => {
  const h = new Heap(Buffer.from('data'))
  h.setPosition(2)
  const bytes = h.toBytes()
  assert.deepEqual(bytes, Buffer.from('data'))
  // Mutating the returned buffer does not affect the heap.
  bytes[0] = 0x5a
  assert.deepEqual(h.toBytes(), Buffer.from('data'))
})

test('equals is content equality, ignoring the cursor', () => {
  const a = new Heap(Buffer.from('same'))
  const b = new Heap(Buffer.from('same'))
  a.setPosition(3) // different cursor
  assert.ok(a.equals(b), 'equality is over the bytes, not the cursor')
  assert.ok(!a.equals(new Heap(Buffer.from('diff'))))
})

test('copy is an independent clone', () => {
  const h = new Heap(Buffer.from('orig'))
  h.setPosition(2)
  const dup = h.copy()
  assert.ok(dup.equals(h))
  // Mutating the copy leaves the original untouched.
  dup.pwriteByte(0, 0x5a)
  assert.deepEqual(h.toBytes(), Buffer.from('orig'))
  assert.deepEqual(dup.toBytes(), Buffer.from([0x5a, 0x72, 0x69, 0x67]))
})

test('toString reports the length', () => {
  assert.equal(new Heap(Buffer.from('hello')).toString(), 'Heap(len=5)')
  assert.equal(new Heap().toString(), 'Heap(len=0)')
})
