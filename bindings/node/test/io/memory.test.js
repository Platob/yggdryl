'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('../..')
const io = yggdryl.io
const { Headers } = yggdryl.headers
const { Heap, Whence, Cursor, Slice, NoChildren } = yggdryl.memory
const { Uri } = yggdryl.uri

// -------------------------------------------------------------------------------------
// Namespace + construction
// -------------------------------------------------------------------------------------

test('the memory namespace exposes Heap, Whence, and NoChildren', () => {
  assert.equal(typeof Heap, 'function')
  assert.equal(typeof NoChildren, 'function')
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

// -------------------------------------------------------------------------------------
// Heap address (uri)
// -------------------------------------------------------------------------------------

test('uri is always the synthetic mem://heap (a heap stores no address)', () => {
  const h = new Heap(Buffer.from('x'))
  assert.ok(h.uri instanceof Uri)
  assert.equal(h.uri.toString(), 'mem://heap') // stable synthetic address
  assert.equal(h.uri.scheme, 'mem')
  assert.equal(h.uri.host, 'heap')
  assert.equal(new Heap().uri.toString(), 'mem://heap')
  // There is deliberately no setter (an anonymous in-memory buffer has no other identity).
  assert.equal(h.setUri, undefined)
  assert.equal(h.withUri, undefined)
})

test('copy is a plain clone (bytes, cursor, headers, mode)', () => {
  const h = new Heap(Buffer.from('orig'))
  const clone = h.copy()
  assert.ok(clone.equals(h))
  clone.pwriteByte(0, 0x5a)
  assert.deepEqual(h.toBytes(), Buffer.from('orig')) // original untouched
})

// -------------------------------------------------------------------------------------
// Heap.cursor() / Heap.window()
// -------------------------------------------------------------------------------------

test('heap.cursor() yields an independent Cursor over a copy', () => {
  const h = new Heap(Buffer.from('hello world'))
  const cur = h.cursor()
  assert.ok(cur instanceof Cursor)
  assert.equal(cur.byteSize(), 11)
  assert.deepEqual(cur.read(5), Buffer.from('hello'))
  assert.equal(cur.position, 5)

  // The cursor is over a copy — writing through it does not affect the source heap.
  cur.setPosition(0)
  assert.equal(cur.write(Buffer.from('HELLO')), 5)
  assert.deepEqual(cur.toBytes(), Buffer.from('HELLO world'))
  assert.deepEqual(h.toBytes(), Buffer.from('hello world')) // original untouched
})

test('heap.window() yields a bounded Slice; OOB throws; writes clamp; copy is independent', () => {
  const h = new Heap(Buffer.from('hello world'))
  const win = h.window(6, 5)
  assert.ok(win instanceof Slice)
  assert.equal(win.byteSize(), 5)
  assert.equal(win.offset, 6)
  assert.deepEqual(win.toBytes(), Buffer.from('world'))
  assert.equal(win.preadByte(0), 'w'.charCodeAt(0))

  // Out of bounds throws a guided error.
  assert.throws(() => h.window(6, 6), /runs past the end/) // 6 + 6 > 11
  assert.throws(() => h.window(6, 6), /11/)

  // A write past the window's end is clamped to the window length.
  assert.equal(win.pwriteByteArray(0, Buffer.from('ABCDEFGH')), 5) // only 5 fit
  assert.deepEqual(win.toBytes(), Buffer.from('ABCDE'))
  assert.deepEqual(h.toBytes(), Buffer.from('hello world')) // window is over a copy
})

// -------------------------------------------------------------------------------------
// Cursor class (direct)
// -------------------------------------------------------------------------------------

test('Cursor: construction, stream + typed round-trips, seek, readToEnd', () => {
  const cur = new Cursor(Buffer.from('abc'))
  assert.equal(cur.byteSize(), 3)
  assert.equal(cur.bitSize(), 24)
  assert.equal(cur.readByte(), 0x61)
  assert.equal(cur.position, 1)

  // Typed writes advance the position; rewind + typed reads round-trip.
  const c2 = new Cursor()
  c2.writeI32(-7)
  c2.writeI64(Math.pow(2, 40)) // below 2^53
  assert.equal(c2.position, 12)
  c2.rewind()
  assert.equal(c2.readI32(), -7)
  assert.equal(c2.readI64(), Math.pow(2, 40))

  // Positioned typed + bit accessors.
  c2.pwriteByte(0, 0xff)
  assert.equal(c2.preadByte(0), 0xff)
  assert.equal(c2.preadBit(0), true)
  assert.equal(c2.preadBit(3), true)

  // seek + readToEnd.
  const c3 = new Cursor(Buffer.from('hello world'))
  assert.equal(c3.seek(Whence.Start, 6), 6)
  assert.deepEqual(c3.readToEnd(), Buffer.from('world'))

  // Read past the end throws.
  assert.throws(() => new Cursor().readByte(), /unexpected end of data/)

  // inner() / toBytes() expose a copy of the wrapped heap.
  assert.ok(c3.inner() instanceof Heap)
  assert.deepEqual(c3.inner().toBytes(), Buffer.from('hello world'))
  assert.deepEqual(c3.toBytes(), Buffer.from('hello world'))
  assert.match(c3.toString(), /^Cursor\(pos=11, len=11\)$/)
})

test('Cursor.over wraps an existing Heap in a cursor over a copy', () => {
  const h = new Heap(Buffer.from('hello world'))
  const cur = Cursor.over(h)
  assert.ok(cur instanceof Cursor)
  assert.equal(cur.byteSize(), 11)
  assert.equal(cur.position, 0)
  assert.deepEqual(cur.read(5), Buffer.from('hello'))

  // The cursor is over a copy — writing through it leaves the source heap untouched.
  cur.rewind()
  cur.write(Buffer.from('HELLO'))
  assert.deepEqual(cur.toBytes(), Buffer.from('HELLO world'))
  assert.deepEqual(h.toBytes(), Buffer.from('hello world'))
})

// -------------------------------------------------------------------------------------
// Slice class (direct)
// -------------------------------------------------------------------------------------

test('Slice.over: bounded reads/writes clamp to the window; OOB throws', () => {
  const h = new Heap(Buffer.from('hello world'))
  const win = Slice.over(h, 0, 5)
  assert.ok(win instanceof Slice)
  assert.equal(win.byteSize(), 5)
  assert.equal(win.offset, 0)
  assert.deepEqual(win.preadByteArray(0, 5), Buffer.from('hello'))
  assert.deepEqual(win.preadByteArray(3, 10), Buffer.from('lo')) // short near the window end
  assert.equal(win.preadByte(0), 0x68) // 'h'

  // Out-of-bounds window throws.
  assert.throws(() => Slice.over(h, 6, 6), /runs past the end/)

  // Clamped write.
  assert.equal(win.pwriteByteArray(3, Buffer.from('ABCDEF')), 2) // only 2 fit before the end
  assert.deepEqual(win.toBytes(), Buffer.from('helAB'))

  // Typed positioned reads over a window.
  const nums = new Heap()
  nums.pwriteI32(0, 123456)
  nums.pwriteI64(4, 7890123456)
  const numWin = Slice.over(nums, 0, 12)
  assert.equal(numWin.preadI32(0), 123456)
  assert.equal(numWin.preadI64(4), 7890123456)
  assert.ok(numWin.inner() instanceof Heap)
  assert.match(numWin.toString(), /^Slice\(offset=0, len=12\)$/)
})

// -------------------------------------------------------------------------------------
// uri delegation through the views
// -------------------------------------------------------------------------------------

test('Cursor and Slice delegate uri to the wrapped source', () => {
  const h = new Heap(Buffer.from('data'))
  assert.equal(h.cursor().uri.toString(), 'mem://heap')
  assert.equal(h.window(0, 2).uri.toString(), 'mem://heap')
  assert.equal(Slice.over(h, 1, 2).uri.host, 'heap')
})

// -------------------------------------------------------------------------------------
// Heap metadata: headers / mode / kind
// -------------------------------------------------------------------------------------

test('heap headers: empty by default; the getter returns a copy; setHeaders writes back', () => {
  const h = new Heap()
  assert.ok(h.headers instanceof Headers)
  assert.ok(h.headers.isEmpty())

  // The getter is a copy — editing it does not write back.
  const copy = h.headers
  copy.insert('X-Edit', '1')
  assert.ok(!h.headers.contains('X-Edit'))

  // setHeaders stores the updated map.
  h.setHeaders(copy)
  assert.equal(h.headers.get('x-edit'), '1')
})

test('withHeaders returns a copy with the map replaced; the original is untouched', () => {
  const h = new Heap(Buffer.from('x'))
  const tagged = h.withHeaders(new Headers().with('Content-Type', 'text/plain'))
  assert.equal(tagged.headers.contentType(), 'text/plain')
  assert.ok(h.headers.isEmpty()) // original untouched
  assert.ok(tagged.equals(h), 'headers are metadata — equality is over the bytes')
})

test('heap mode defaults to ReadWrite; setMode / withMode; kind is Heap', () => {
  const h = new Heap()
  assert.equal(h.mode, io.IOMode.ReadWrite)
  assert.equal(h.kind, io.IOKind.Heap)

  h.setMode(io.IOMode.Read)
  assert.equal(h.mode, io.IOMode.Read)

  const appendOnly = new Heap().withMode(io.IOMode.Append)
  assert.equal(appendOnly.mode, io.IOMode.Append)
  assert.equal(new Heap().mode, io.IOMode.ReadWrite) // withMode did not mutate a default
})

test('Cursor and Slice delegate headers / mode / kind to the wrapped source', () => {
  const src = new Heap(Buffer.from('data'))
    .withMode(io.IOMode.Read)
    .withHeaders(new Headers().with('Content-Type', 'text/plain'))

  const cur = src.cursor()
  assert.equal(cur.mode, io.IOMode.Read)
  assert.equal(cur.kind, io.IOKind.Heap)
  assert.equal(cur.headers.contentType(), 'text/plain')

  const win = src.window(0, 2)
  assert.equal(win.mode, io.IOMode.Read)
  assert.equal(win.kind, io.IOKind.Heap)
  assert.equal(win.headers.contentType(), 'text/plain')

  assert.equal(Slice.over(src, 1, 2).mode, io.IOMode.Read)
})

// -------------------------------------------------------------------------------------
// UTF-8 text accessors
// -------------------------------------------------------------------------------------

test('positioned preadUtf8 / pwriteUtf8 round-trip; a cut multi-byte char throws', () => {
  const h = new Heap()
  assert.equal(h.pwriteUtf8(0, 'héllo'), 6) // é is 2 bytes — byte count, not chars
  assert.equal(h.preadUtf8(0, 6), 'héllo')
  assert.equal(h.preadUtf8(0, 100), 'héllo') // clamped near the end, like preadByteArray
  assert.equal(h.preadUtf8(0, 1), 'h')

  const cut = new Heap()
  cut.pwriteUtf8(0, 'é')
  assert.throws(() => cut.preadUtf8(0, 1), /invalid UTF-8/) // cuts the 2-byte char in half
  assert.throws(() => cut.preadUtf8(0, 1), /pread_byte_array/) // the guided fix
})

test('heap stream readUtf8 / writeUtf8 advance the cursor', () => {
  const h = new Heap()
  assert.equal(h.writeUtf8('héllo'), 6)
  assert.equal(h.position, 6)
  h.rewind()
  assert.equal(h.readUtf8(6), 'héllo')
  assert.equal(h.position, 6)
})

test('Cursor readUtf8 / writeUtf8 mirror the heap stream forms', () => {
  const cur = new Cursor()
  assert.equal(cur.writeUtf8('wörld'), 6)
  cur.rewind()
  assert.equal(cur.readUtf8(6), 'wörld')
  assert.equal(cur.position, 6)

  const bad = new Cursor()
  bad.writeUtf8('é')
  bad.rewind()
  assert.throws(() => bad.readUtf8(1), /invalid UTF-8/)
  assert.equal(bad.position, 0, 'a failed decode leaves the cursor put')
})

test('Cursor positioned preadUtf8 / pwriteUtf8 never move the position', () => {
  const cur = new Cursor(Buffer.from('xxxx'))
  cur.setPosition(2)
  assert.equal(cur.pwriteUtf8(0, 'héllo'), 6)
  assert.equal(cur.preadUtf8(0, 6), 'héllo')
  assert.equal(cur.position, 2, 'positioned utf8 accessors leave the position put')
  assert.throws(() => cur.preadUtf8(1, 1), /invalid UTF-8/) // cuts é in half
})

test('Slice preadUtf8 decodes within the window (clamped); no pwriteUtf8 exists', () => {
  const h = new Heap()
  h.pwriteUtf8(0, 'say héllo') // 10 bytes; é spans bytes 5-6
  const win = h.window(4, 5) // "héll" — h + é(2 bytes) + l + l
  assert.equal(win.preadUtf8(0, 100), 'héll') // clamped to the 5-byte window
  assert.throws(() => win.preadUtf8(1, 1), /invalid UTF-8/) // cuts é in half

  // The window is fixed-length, so it deliberately has no pwriteUtf8 (matching Python).
  assert.equal(win.pwriteUtf8, undefined)
})

// -------------------------------------------------------------------------------------
// Bit offsets are i64 (bits past 2^32 stay addressable; negatives throw)
// -------------------------------------------------------------------------------------

test('negative bit offsets throw a guided error naming the offending value', () => {
  const h = new Heap(Buffer.from([0b0000_0001]))
  assert.equal(h.preadBit(0), true) // non-negative still works
  assert.throws(() => h.preadBit(-1), /invalid bit offset -1/)
  assert.throws(() => h.preadBit(-1), /non-negative/) // the guided fix
  assert.throws(() => h.pwriteBit(-5, true), /invalid bit offset -5/)

  const cur = new Cursor(Buffer.from([0xff]))
  assert.equal(cur.preadBit(7), true)
  assert.throws(() => cur.preadBit(-1), /invalid bit offset -1/)
  assert.throws(() => cur.pwriteBit(-1, true), /invalid bit offset -1/)
})

// -------------------------------------------------------------------------------------
// Bulk typed arrays (1000 elements crosses the 256-element staging chunk)
// -------------------------------------------------------------------------------------

test('preadI32Array / pwriteI32Array round-trip 1000 values; short data throws', () => {
  const values = Array.from({ length: 1000 }, (_, i) => (i % 2 ? -1 : 1) * i * 1000)
  const h = new Heap()
  h.pwriteI32Array(0, values)
  assert.equal(h.byteSize(), 4000)
  assert.deepEqual(h.preadI32Array(0, 1000), values)
  assert.equal(h.preadI32(4), -1000) // little-endian, element-addressable

  assert.throws(() => h.preadI32Array(3999, 2), /unexpected end of data/)
  assert.deepEqual(new Heap().preadI32Array(0, 0), []) // empty read is fine
})

test('preadI64Array / pwriteI64Array round-trip 1000 values below 2^53', () => {
  const values = Array.from({ length: 1000 }, (_, i) => i * 2 ** 40 + i) // max ~2^50
  const h = new Heap()
  h.pwriteI64Array(0, values)
  assert.equal(h.byteSize(), 8000)
  assert.deepEqual(h.preadI64Array(0, 1000), values)
  assert.equal(h.preadI64(8), 2 ** 40 + 1)

  assert.throws(() => h.preadI64Array(0, 1001), /unexpected end of data/)
})

test('bulk reads reject a hostile count fast, before allocating', () => {
  const h = new Heap(Buffer.from('tiny'))
  const start = process.hrtime.bigint()
  assert.throws(() => h.preadI32Array(0, 2_000_000_000), /unexpected end of data/)
  assert.throws(() => h.preadI32Array(0, 2_000_000_000), /8000000000 bytes/) // count * 4 named
  assert.throws(() => h.preadI64Array(0, 2_000_000_000), /unexpected end of data/)
  const elapsedMs = Number(process.hrtime.bigint() - start) / 1e6
  assert.ok(elapsedMs < 1000, `the guard must fail fast, took ${elapsedMs}ms`)
})

// -------------------------------------------------------------------------------------
// Repeated-value fills
// -------------------------------------------------------------------------------------

test('pwriteByteRepeat fills without materializing the array (gap zero-filled)', () => {
  const h = new Heap()
  h.pwriteByteRepeat(2, 0xab, 5)
  assert.deepEqual(h.toBytes(), Buffer.from([0, 0, 0xab, 0xab, 0xab, 0xab, 0xab]))

  const big = new Heap()
  big.pwriteByteRepeat(0, 0x77, 5000) // crosses the staging chunk
  assert.equal(big.byteSize(), 5000)
  assert.equal(big.preadByte(4999), 0x77)
})

test('pwriteI32Repeat / pwriteI64Repeat fill typed runs past the staging chunk', () => {
  const r32 = new Heap()
  r32.pwriteI32Repeat(0, -1, 300) // 300 > the 256-element chunk
  assert.equal(r32.byteSize(), 1200)
  assert.ok(r32.preadI32Array(0, 300).every((v) => v === -1))

  const r64 = new Heap()
  r64.pwriteI64Repeat(0, 2 ** 40 + 7, 300)
  assert.equal(r64.byteSize(), 2400)
  assert.ok(r64.preadI64Array(0, 300).every((v) => v === 2 ** 40 + 7))
})

// -------------------------------------------------------------------------------------
// Heap byte codec (Serializable)
// -------------------------------------------------------------------------------------

test('serializeBytes / deserializeBytes round-trip the stored bytes (metadata excluded)', () => {
  const h = new Heap(Buffer.from('hello')).withMode(io.IOMode.Read)
  h.setPosition(3)

  const frame = h.serializeBytes()
  assert.deepEqual(frame, Buffer.from('hello')) // the value form IS the stored bytes

  const back = Heap.deserializeBytes(frame)
  assert.ok(back instanceof Heap)
  assert.ok(back.equals(h)) // same identity equals uses
  assert.equal(back.position, 0) // cursor is transient — not carried
  assert.equal(back.mode, io.IOMode.ReadWrite) // metadata is not serialized

  const empty = Heap.deserializeBytes(Buffer.alloc(0))
  assert.ok(empty.isEmpty())
})

// -------------------------------------------------------------------------------------
// Capacity family: checked reserves, ensure, shrink, spare
// -------------------------------------------------------------------------------------

test('capacity family: checked reserves, ensure, shrink, spare', () => {
  const h = Heap.withCapacity(64)
  assert.ok(h.spareCapacity() >= 64)
  h.pwriteByteArray(0, Buffer.alloc(16))
  assert.equal(h.spareCapacity(), h.capacity() - 16)

  h.reserveExact(100)
  assert.ok(h.capacity() >= 116)

  // Checked reserves: a hostile size throws the guided error, never aborts the process.
  h.tryReserve(1024)
  h.tryReserveExact(2048)
  assert.throws(() => h.tryReserve(Number.MAX_SAFE_INTEGER), /reserve less/)
  assert.throws(() => h.tryEnsureCapacity(Number.MAX_SAFE_INTEGER), /reserve less/)
  // Still fully usable afterwards.
  h.pwriteUtf8(0, 'alive')
  assert.equal(h.preadUtf8(0, 5), 'alive')

  // ensureCapacity targets a total and never shrinks.
  h.ensureCapacity(8192)
  assert.ok(h.capacity() >= 8192)
  const cap = h.capacity()
  h.ensureCapacity(16)
  assert.equal(h.capacity(), cap)

  // shrink releases spare room (contents untouched).
  h.shrinkTo(64)
  h.shrinkToFit()
  assert.ok(h.capacity() <= cap)
  assert.equal(h.preadUtf8(0, 5), 'alive')
})

// -------------------------------------------------------------------------------------
// IOBase predicates: isFile / isDir / exists
// -------------------------------------------------------------------------------------

test('a live heap exists although it is neither file nor directory', () => {
  const h = new Heap(Buffer.from('x'))
  assert.equal(h.isFile(), false) // kind is IOKind.Heap, not a file
  assert.equal(h.isDir(), false)
  assert.equal(h.exists(), true) // a live in-memory buffer always exists
  assert.equal(new Heap().exists(), true) // even empty
})

test('Cursor and Slice predicates forward the wrapped source, not a re-derivation', () => {
  const cur = new Cursor(Buffer.from('x'))
  assert.equal(cur.isFile(), false)
  assert.equal(cur.isDir(), false)
  assert.equal(cur.exists(), true) // forwards the heap's own notion, like the core

  const win = Slice.over(new Heap(Buffer.from('abc')), 0, 2)
  assert.equal(win.isFile(), false)
  assert.equal(win.isDir(), false)
  assert.equal(win.exists(), true)
})

// -------------------------------------------------------------------------------------
// The graph surface — Heap / Cursor / Slice are leaf nodes
// -------------------------------------------------------------------------------------

test('a heap is a leaf node: empty name, null parent, empty streamed ls, empty children', () => {
  const h = new Heap(Buffer.from('x'))
  assert.equal(h.name, '') // mem://heap has no path segment to take a name from
  assert.equal(h.parent(), null) // a leaf has no parent

  // ls() is a real (always-empty) iterable — streamed, never a pre-collected array.
  const entries = h.ls()
  assert.ok(entries instanceof NoChildren)
  assert.ok(!Array.isArray(entries))
  assert.ok(Symbol.iterator in entries)
  const iterator = entries[Symbol.iterator]()
  assert.equal(typeof iterator.next, 'function')
  assert.equal(iterator.next().done, true) // a leaf streams nothing
  assert.deepEqual([...h.ls()], [])
  assert.deepEqual([...h.ls(true)], []) // recursive changes nothing on a leaf
  assert.equal(h.ls().toString(), 'NoChildren(<empty>)')

  // children() is the collected convenience — an empty array, not an iterable.
  assert.deepEqual(h.children(), [])
})

test('removing a heap is the guided refusal naming the fix', () => {
  const h = new Heap()
  assert.throws(() => h.rm(), /rm needs a removable backing/)
  assert.throws(() => h.rm(), /this source has none/)
  assert.throws(() => h.rm(), /LocalIO/) // the fix: address a filesystem node instead
  assert.throws(() => h.rmfile(), /rmfile needs a removable backing/)
  assert.throws(() => h.rmdir(), /rmdir needs a removable backing/)
})

test('Cursor and Slice carry the same leaf graph surface', () => {
  const cur = new Cursor(Buffer.from('abc'))
  assert.equal(cur.name, '')
  assert.equal(cur.parent(), null)
  assert.ok(cur.ls() instanceof NoChildren)
  assert.deepEqual([...cur.ls()], [])
  assert.deepEqual([...cur.ls(true)], [])
  assert.deepEqual(cur.children(), [])
  assert.throws(() => cur.rm(), /removable backing/)
  assert.throws(() => cur.rmfile(), /LocalIO/)

  const win = Slice.over(new Heap(Buffer.from('abc')), 0, 2)
  assert.equal(win.name, '')
  assert.equal(win.parent(), null)
  assert.ok(win.ls() instanceof NoChildren)
  assert.deepEqual([...win.ls()], [])
  assert.deepEqual(win.children(), [])
  assert.throws(() => win.rmdir(), /removable backing/)
  assert.throws(() => win.rm(), /LocalIO/)
})

// -------------------------------------------------------------------------------------
// Heap.join — a heap is a leaf for discovery but still addressable (join composes an
// address; parent navigates back). The named mirror of Python's `__truediv__`.
// -------------------------------------------------------------------------------------

test('join composes a child address; the child is an independent buffer; parent navigates back', () => {
  const root = new Heap()
  const child = root.join('logs/app.bin')
  assert.ok(child instanceof Heap)
  assert.equal(child.uri.toString(), 'mem://heap/logs/app.bin')
  assert.equal(child.name, 'app.bin') // the last address segment, percent-decoded

  // The child is an independent, writable buffer — the parent heap stays empty.
  assert.equal(child.pwriteUtf8(0, 'log line'), 8)
  assert.equal(child.preadUtf8(0, 8), 'log line')
  assert.equal(root.byteSize(), 0) // writing the child never touched the root
  assert.equal(child.uri.toString(), 'mem://heap/logs/app.bin') // address survives the write

  // parent() navigates back up the composed address.
  const logs = child.parent()
  assert.ok(logs instanceof Heap)
  assert.equal(logs.uri.toString(), 'mem://heap/logs')
  assert.equal(logs.parent().uri.toString(), 'mem://heap') // grandparent is the root

  // The bare mem://heap root has no parent (a joined child's chain bottoms out at null).
  assert.equal(new Heap().parent(), null)
  assert.equal(logs.parent().parent(), null)
})

test('a spaced join segment percent-encodes in the composed address', () => {
  assert.equal(new Heap().join('my dir/f').uri.toString(), 'mem://heap/my%20dir/f')
})

