'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('../..')
const io = yggdryl.io
const { DataTypeId } = yggdryl.datatype_id
const { Headers } = yggdryl.headers
const { Heap, Whence, Cursor, Slice, NoChildren } = yggdryl.memory
const { Uri } = yggdryl.uri
const { MimeType } = yggdryl.mimetype
const { MediaType } = yggdryl.mediatype
const { Gzip, Zstd, Lzma, codecFor } = yggdryl.compression

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

test('parents() lists a heap node ancestors nearest-first; leaves and roots are empty', () => {
  const node = new Heap().join('a/b/c')
  const uris = node.parents().map((p) => p.uri.toString())
  assert.deepEqual(uris, ['mem://heap/a/b', 'mem://heap/a', 'mem://heap'])
  assert.ok(node.parents().every((p) => p instanceof Heap))

  // The bare mem://heap root has no ancestors — the collected `parent()` chain is empty.
  assert.deepEqual(new Heap().parents(), [])

  // Cursor and Slice are leaves — parent() is null, so parents() is always empty.
  assert.deepEqual(new Cursor(Buffer.from('x')).parents(), [])
  assert.deepEqual(Slice.over(new Heap(Buffer.from('abc')), 0, 2).parents(), [])
})

// -------------------------------------------------------------------------------------
// media type inference (IOBase.mimeType / mediaType / ensureContentType)
// -------------------------------------------------------------------------------------

test('a heap infers its media type from headers, else the octet-stream fallback', () => {
  const heap = new Heap()
  // No headers and no address extension -> the octet-stream fallback (never null).
  assert.ok(heap.mimeType() instanceof MimeType)
  assert.ok(heap.mimeType().isOctetStream())
  assert.ok(heap.mediaType() instanceof MediaType)
  assert.deepEqual(heap.mediaType().essences(), ['application/octet-stream'])

  // A declared Content-Type wins.
  heap.setHeaders(new Headers().with('Content-Type', 'application/json'))
  assert.equal(heap.mimeType().essence, 'application/json')

  // Content-Type + Content-Encoding compose into the layered media type.
  const tarred = new Heap()
  const meta = new Headers().with('Content-Type', 'application/x-tar')
  meta.setContentEncoding('gzip')
  tarred.setHeaders(meta)
  assert.deepEqual(tarred.mediaType().essences(), ['application/x-tar', 'application/gzip'])
})

test('ensureContentType memoizes the inferred type into the headers', () => {
  const heap = new Heap()
  // No Content-Type yet: it infers (octet-stream here) and stores it.
  assert.equal(heap.ensureContentType().essence, 'application/octet-stream')
  assert.equal(heap.headers.get('Content-Type'), 'application/octet-stream')

  // A pre-set Content-Type is returned unchanged (never overwritten).
  const typed = new Heap()
  typed.setHeaders(new Headers().with('Content-Type', 'image/png'))
  assert.equal(typed.ensureContentType().essence, 'image/png')
})

test('Cursor and Slice delegate the media type to their wrapped source', () => {
  const heap = new Heap(Buffer.from('hello world'))
  heap.setHeaders(new Headers().with('Content-Type', 'text/plain'))

  const cursor = heap.cursor()
  assert.equal(cursor.mimeType().essence, 'text/plain')
  assert.deepEqual(cursor.mediaType().essences(), ['text/plain'])

  const slice = heap.window(0, 5)
  assert.equal(slice.mimeType().essence, 'text/plain')

  // A bare view with no headers falls back to octet-stream.
  assert.ok(new Cursor(Buffer.from('x')).mimeType().isOctetStream())
})

// -------------------------------------------------------------------------------------
// magic inference + compression (IOBase.inferMimeType / inferMediaType / compression /
// compressWith / decompressWith / decompress)
// -------------------------------------------------------------------------------------

const PNG_MAGIC = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])

test('inferMimeType reads the magic bytes without moving the cursor', () => {
  const heap = new Heap(PNG_MAGIC)
  heap.setPosition(3) // park the cursor mid-stream
  assert.equal(heap.inferMimeType().essence, 'image/png') // positioned head read
  assert.equal(heap.position, 3) // the cursor never moved

  // No magic match -> falls back to the declared/address mimeType (octet-stream here).
  assert.ok(new Heap(Buffer.from('plain bytes')).inferMimeType().isOctetStream())
})

test('inferMediaType peels compression layers by recursive magic', () => {
  // A gzip stream whose decompressed head is itself a recognizable magic (PNG).
  const gz = new Gzip().compress(PNG_MAGIC)
  const heap = new Heap(gz)
  // Outer magic is gzip; the peeled inner head is PNG.
  assert.deepEqual(heap.inferMediaType().essences(), ['application/gzip', 'image/png'])

  // A plain, non-compression source is a single layer.
  assert.deepEqual(new Heap(PNG_MAGIC).inferMediaType().essences(), ['image/png'])
})

test('compression() resolves a codec from the source media type, else null', () => {
  const gzipped = new Heap(new Gzip().compress(Buffer.from('data')))
  gzipped.setHeaders(new Headers().with('Content-Type', 'application/gzip'))
  assert.ok(gzipped.compression() instanceof Gzip)

  // No compression content type -> null.
  assert.equal(new Heap(Buffer.from('x')).compression(), null)
})

test('decompress() uses the codec inferred from the content-type header', () => {
  const payload = Buffer.from('hello '.repeat(200))

  // A gzip heap addressed by its Content-Type decompresses to the original.
  const gz = new Heap(new Gzip().compress(payload))
  gz.setHeaders(new Headers().with('Content-Type', 'application/gzip'))
  assert.deepEqual(gz.decompress(), payload)

  // A zstd heap, same path.
  const zs = new Heap(new Zstd().compress(payload))
  zs.setHeaders(new Headers().with('Content-Type', 'application/zstd'))
  assert.deepEqual(zs.decompress(), payload)

  // A non-compression source throws the guided compression error.
  assert.throws(() => new Heap(payload).decompress(), /compression/)
})

test('compressWith / decompressWith run an explicit codec over the whole source', () => {
  const payload = Buffer.from('the quick brown fox '.repeat(50))
  const heap = new Heap(payload)

  const gzip = new Gzip()
  const packed = heap.compressWith(gzip)
  assert.ok(packed.length < payload.length)
  // Round-trip: rebuild a heap over the packed bytes and decompress it back.
  assert.deepEqual(new Heap(packed).decompressWith(gzip), payload)

  // Any of the four codec classes is accepted (the Either4 codec argument).
  const xz = new Lzma()
  assert.deepEqual(new Heap(heap.compressWith(xz)).decompressWith(xz), payload)

  // A codec resolved via codecFor works interchangeably.
  assert.deepEqual(new Heap(packed).decompressWith(codecFor('application/gzip')), payload)
})

test('Cursor and Slice inherit the compression surface from their source', () => {
  const payload = Buffer.from('cursor payload '.repeat(30))
  const gz = new Gzip().compress(payload)

  // The content type is set on the Heap; a cursor over it delegates the media type (and so
  // the inferred codec) to the wrapped source.
  const source = new Heap(gz)
  source.setHeaders(new Headers().with('Content-Type', 'application/gzip'))
  const cursor = source.cursor()
  assert.ok(cursor.compression() instanceof Gzip)
  assert.deepEqual(cursor.decompress(), payload)

  // A Slice window over the whole gzip stream decompresses with an explicit codec.
  const slice = new Heap(gz).window(0, gz.length)
  assert.deepEqual(slice.decompressWith(new Gzip()), payload)
})

// -------------------------------------------------------------------------------------
// truncate + contentLength
// -------------------------------------------------------------------------------------

test('truncate shrinks (drops the tail) and grows (zero-fills)', () => {
  const h = new Heap(Buffer.from('hello world'))
  h.truncate(5)
  assert.deepEqual(h.toBytes(), Buffer.from('hello'))
  h.truncate(8) // grow zero-fills
  assert.deepEqual(h.toBytes(), Buffer.from([0x68, 0x65, 0x6c, 0x6c, 0x6f, 0, 0, 0]))
  h.truncate(0)
  assert.ok(h.isEmpty())
})

test('contentLength reads the header when present, else falls back to byteSize', () => {
  const h = new Heap(Buffer.from('abcd'))
  assert.equal(h.contentLength(), 4) // no header — the live byteSize
  h.setHeaders(new Headers().with('Content-Length', '99'))
  assert.equal(h.contentLength(), 99) // the cached header is authoritative

  // Cursor and Slice delegate to the wrapped source.
  const src = new Heap(Buffer.from('abcdef'))
  assert.equal(src.cursor().contentLength(), 6)
  assert.equal(src.window(0, 2).contentLength(), 2)
})

// -------------------------------------------------------------------------------------
// In-place compression (compressInPlace / decompressInPlace)
// -------------------------------------------------------------------------------------

test('compressInPlace + decompressInPlace round-trip and sync the content-type header', () => {
  const original = Buffer.from('in-place payload '.repeat(40))
  const h = new Heap(original)

  // An explicit codec packs the whole source and stamps the Content-Type.
  h.compressInPlace(new Gzip())
  assert.ok(h.byteSize() < original.length)
  assert.equal(h.headers.contentType(), 'application/gzip')

  // decompressInPlace infers the codec from the (now gzip) media type and restores the bytes.
  h.decompressInPlace()
  assert.deepEqual(h.toBytes(), original)

  // With no codec and a non-compression media type, compressInPlace throws the guided error.
  assert.throws(() => new Heap(Buffer.from('x')).compressInPlace(), /codec/)
})

// -------------------------------------------------------------------------------------
// Cross-source copy (copyFrom / pwriteFrom)
// -------------------------------------------------------------------------------------

test('copyFrom overwrites with all of the source bytes (truncating to match)', () => {
  const dst = new Heap(Buffer.from('old data here'))
  const src = new Heap(Buffer.from('new'))
  assert.equal(dst.copyFrom(src), 3)
  assert.deepEqual(dst.toBytes(), Buffer.from('new')) // truncated to the source length
})

test('pwriteFrom copies a positioned window of the source into this heap', () => {
  const dst = new Heap(Buffer.from('....'))
  const src = new Heap(Buffer.from('ABCDEF'))
  assert.equal(dst.pwriteFrom(1, src, 2, 3), 3) // src[2..5] = 'CDE' at offset 1
  assert.deepEqual(dst.toBytes(), Buffer.from('.CDE'))

  // A length past the source's end transfers only what remains.
  assert.equal(dst.pwriteFrom(0, src, 4, 10), 2) // only 'EF' remain from offset 4
  assert.deepEqual(dst.toBytes(), Buffer.from('EFDE'))
})

// -------------------------------------------------------------------------------------
// Bulk typed arrays for the unsigned + floating widths (u16/u32/u64/f32/f64) + repeats
// -------------------------------------------------------------------------------------

test('u16 / u32 / u64 / f32 / f64 bulk arrays round-trip 300 values (crossing the chunk)', () => {
  const h = new Heap()

  const u16s = Array.from({ length: 300 }, (_, i) => (i * 7) % 65536)
  h.pwriteU16Array(0, u16s)
  assert.deepEqual(h.preadU16Array(0, 300), u16s)

  const u32s = Array.from({ length: 300 }, (_, i) => i * 100000)
  h.pwriteU32Array(0, u32s)
  assert.deepEqual(h.preadU32Array(0, 300), u32s)

  // u64 crosses as a JS number (i64) — the full value is carried without truncation (< 2^53).
  const u64s = Array.from({ length: 300 }, (_, i) => i * 2 ** 40 + i)
  h.pwriteU64Array(0, u64s)
  assert.deepEqual(h.preadU64Array(0, 300), u64s)

  // f32 values are f32-exact (multiples of 0.5), so the f64 round-trip is lossless.
  const f32s = Array.from({ length: 300 }, (_, i) => (i % 2 ? -1 : 1) * (i * 0.5))
  h.pwriteF32Array(0, f32s)
  assert.deepEqual(h.preadF32Array(0, 300), f32s)

  const f64s = Array.from({ length: 300 }, (_, i) => i * 0.1 - 5)
  h.pwriteF64Array(0, f64s)
  assert.deepEqual(h.preadF64Array(0, 300), f64s)

  // Hostile counts fail fast (the shared guard), before allocating.
  assert.throws(() => new Heap(Buffer.from('t')).preadU64Array(0, 2_000_000_000), /unexpected end of data/)
  assert.throws(() => new Heap(Buffer.from('t')).preadF32Array(0, 2_000_000_000), /unexpected end of data/)
})

test('u16 / u32 / u64 / f32 / f64 repeat fills run past the staging chunk', () => {
  const h = new Heap()
  h.pwriteU16Repeat(0, 0xabcd, 300)
  assert.ok(h.preadU16Array(0, 300).every((v) => v === 0xabcd))

  h.pwriteU32Repeat(0, 4000000000, 300)
  assert.ok(h.preadU32Array(0, 300).every((v) => v === 4000000000))

  h.pwriteU64Repeat(0, 2 ** 40 + 7, 10)
  assert.equal(h.preadU64Array(0, 1)[0], 2 ** 40 + 7)

  h.pwriteF32Repeat(0, -2.25, 10)
  assert.ok(h.preadF32Array(0, 10).every((v) => v === -2.25))

  h.pwriteF64Repeat(0, 3.5, 10)
  assert.ok(h.preadF64Array(0, 10).every((v) => v === 3.5))
})

// -------------------------------------------------------------------------------------
// Line-oriented reads (readline / readlines)
// -------------------------------------------------------------------------------------

test('readline / readlines strip the terminator; a blank line is "" but advances; EOF is ""', () => {
  const h = new Heap(Buffer.from('first\nsecond'))
  assert.equal(h.readline(), 'first') // trailing \n stripped
  assert.equal(h.readline(), 'second') // last line, no terminator
  assert.equal(h.readline(), '') // now at EOF (returns "" without advancing)
  assert.equal(h.position, h.byteSize()) // a second EOF read stays put
  assert.equal(h.readline(), '')
  h.rewind()
  assert.deepEqual(h.readlines(), ['first', 'second'])

  // A blank line returns "" but still advances, so it is kept (distinct from EOF).
  const blank = new Heap(Buffer.from('a\n\nb\n'))
  assert.deepEqual(blank.readlines(), ['a', '', 'b'])

  // Cursor mirrors the same line stream.
  const cur = new Cursor(Buffer.from('x\ny\n'))
  assert.equal(cur.readline(), 'x')
  assert.deepEqual(cur.readlines(), ['y'])
})

test('readline strips CRLF and is CSV-quote-aware (a quoted newline does not split)', () => {
  // A CRLF terminator is stripped whole.
  const crlf = new Heap(Buffer.from('alpha\r\nbeta\r\n'))
  assert.deepEqual(crlf.readlines(), ['alpha', 'beta'])

  // A \n inside a double-quoted field is part of the record, not a line break.
  const csv = new Heap(Buffer.from('a,"x\ny",b\nnext'))
  assert.equal(csv.readline(), 'a,"x\ny",b') // the quoted newline is kept in the record
  assert.equal(csv.readline(), 'next') // the real line break ended the previous record
  assert.equal(csv.readline(), '') // EOF

  // The same on a Cursor.
  const cur = new Cursor(Buffer.from('one\r\n"two\nlines"\r\nthree'))
  assert.deepEqual(cur.readlines(), ['one', '"two\nlines"', 'three'])
})

// -------------------------------------------------------------------------------------
// The rm family accepts existOk (a heap always refuses regardless)
// -------------------------------------------------------------------------------------

test('rm accepts an existOk flag; a heap still refuses either way', () => {
  const h = new Heap()
  assert.throws(() => h.rm(), /removable backing/)
  assert.throws(() => h.rm(false), /removable backing/)
  assert.throws(() => h.rm(true), /removable backing/)
})

// -------------------------------------------------------------------------------------
// Type-inference constructors (fromIo) + the lines() alias
// -------------------------------------------------------------------------------------

test('Heap.fromIo infers the input type: string, Uint8Array/Buffer, or another Heap', () => {
  assert.deepEqual(Heap.fromIo('héllo').toBytes(), Buffer.from('héllo')) // UTF-8 bytes
  assert.deepEqual(Heap.fromIo(Buffer.from([1, 2, 3])).toBytes(), Buffer.from([1, 2, 3]))
  assert.deepEqual(Heap.fromIo(Uint8Array.of(4, 5, 6)).toBytes(), Buffer.from([4, 5, 6]))

  const src = new Heap(Buffer.from('clone me'))
  const dup = Heap.fromIo(src)
  assert.ok(dup.equals(src))
  dup.pwriteByte(0, 0x5a) // independent copy
  assert.deepEqual(src.toBytes(), Buffer.from('clone me'))
})

test('Cursor.fromIo infers the input type and carries a source heap position', () => {
  assert.deepEqual(Cursor.fromIo('abc').toBytes(), Buffer.from('abc'))
  assert.deepEqual(Cursor.fromIo(Uint8Array.of(7, 8)).toBytes(), Buffer.from([7, 8]))

  // A source Heap's cursor position becomes the new cursor's start (the source's tell).
  const src = new Heap(Buffer.from('hello world'))
  src.setPosition(6)
  const cur = Cursor.fromIo(src)
  assert.equal(cur.position, 6)
  assert.deepEqual(cur.readToEnd(), Buffer.from('world'))
})

test('lines() is the array alias of readlines() on Heap and Cursor', () => {
  assert.deepEqual(new Heap(Buffer.from('a\nb\n')).lines(), ['a', 'b'])
  assert.deepEqual(new Cursor(Buffer.from('x\ny')).lines(), ['x', 'y'])
})

// -------------------------------------------------------------------------------------
// All native scalar widths (pread/pwrite) — number widths, BigInt widths, f32 via f64
// -------------------------------------------------------------------------------------

test('Heap: every native scalar width round-trips (pread/pwrite)', () => {
  const h = new Heap()
  h.pwriteI8(0, -5)
  h.pwriteU8(1, 200)
  h.pwriteI16(2, -12345)
  h.pwriteU16(4, 60000)
  h.pwriteU32(8, 4000000000)
  h.pwriteU64(16, 12345678901234n) // BigInt
  h.pwriteI128(32, -123456789012345678901234567890n) // BigInt
  h.pwriteU128(48, 340282366920938463463374607431768211455n) // max u128, BigInt
  h.pwriteF32(64, 1.5) // exactly representable in f32
  h.pwriteF64(68, Math.PI)

  assert.equal(h.preadI8(0), -5)
  assert.equal(h.preadU8(1), 200)
  assert.equal(h.preadI16(2), -12345)
  assert.equal(h.preadU16(4), 60000)
  assert.equal(h.preadU32(8), 4000000000)
  assert.equal(h.preadU64(16), 12345678901234n)
  assert.equal(h.preadI128(32), -123456789012345678901234567890n)
  assert.equal(h.preadU128(48), 340282366920938463463374607431768211455n)
  assert.equal(h.preadF32(64), 1.5)
  assert.equal(h.preadF64(68), Math.PI)
})

test('Heap: bulk i8 / i16 arrays + repeat (number arrays)', () => {
  const h = new Heap()
  h.pwriteI8Array(0, [-1, 2, -3])
  assert.deepEqual(h.preadI8Array(0, 3), [-1, 2, -3])
  h.pwriteI8Repeat(0, -7, 4)
  assert.deepEqual(h.preadI8Array(0, 4), [-7, -7, -7, -7])

  h.pwriteI16Array(0, [-1000, 1000, -2000])
  assert.deepEqual(h.preadI16Array(0, 3), [-1000, 1000, -2000])
  h.pwriteI16Repeat(0, 9, 3)
  assert.deepEqual(h.preadI16Array(0, 3), [9, 9, 9])
})

test('Heap: bulk i128 / u128 arrays are BigInt[]', () => {
  const h = new Heap()
  h.pwriteI128Array(0, [-5n, 7n, -9n])
  assert.deepEqual(h.preadI128Array(0, 3), [-5n, 7n, -9n])
  h.pwriteI128Repeat(0, -3n, 2)
  assert.deepEqual(h.preadI128Array(0, 2), [-3n, -3n])

  h.pwriteU128Array(0, [1n, 2n, 340282366920938463463374607431768211455n])
  assert.deepEqual(h.preadU128Array(0, 3), [1n, 2n, 340282366920938463463374607431768211455n])
  h.pwriteU128Repeat(0, 8n, 2)
  assert.deepEqual(h.preadU128Array(0, 2), [8n, 8n])
})

test('Heap: cursor read/write for every native width', () => {
  const h = new Heap()
  h.writeI8(-1)
  h.writeU8(255)
  h.writeI16(-30000)
  h.writeU16(65535)
  h.writeU32(4294967295)
  h.writeU64(9007199254740993n) // > 2^53, exact only via BigInt
  h.writeI128(-42n)
  h.writeU128(42n)
  h.writeF32(1.5)
  h.writeF64(Math.PI)
  h.rewind()
  assert.equal(h.readI8(), -1)
  assert.equal(h.readU8(), 255)
  assert.equal(h.readI16(), -30000)
  assert.equal(h.readU16(), 65535)
  assert.equal(h.readU32(), 4294967295)
  assert.equal(h.readU64(), 9007199254740993n)
  assert.equal(h.readI128(), -42n)
  assert.equal(h.readU128(), 42n)
  assert.equal(h.readF32(), 1.5)
  assert.equal(h.readF64(), Math.PI)
})

test('Heap.moveInto moves the bytes into dst and empties the source', () => {
  const src = new Heap(Buffer.from('relocate me'))
  const dst = new Heap()
  assert.equal(src.moveInto(dst), 11)
  assert.deepEqual(dst.toBytes(), Buffer.from('relocate me'))
  assert.equal(src.byteSize(), 0)
})

test('Cursor: scalar pread/pwrite and cursor read/write for new widths', () => {
  const c = new Cursor()
  c.writeI16(-30000)
  c.writeU128(42n)
  c.rewind()
  assert.equal(c.readI16(), -30000)
  assert.equal(c.readU128(), 42n)

  // The positioned scalar accessors work independently of the cursor.
  c.pwriteU32(0, 123456)
  assert.equal(c.preadU32(0), 123456)
  c.pwriteU64(8, 77n)
  assert.equal(c.preadU64(8), 77n)
})

test('Slice: read-only native scalar reads (a fixed window has no typed writes)', () => {
  const h = new Heap()
  h.pwriteI16(0, -12345)
  h.pwriteU64(2, 999n)
  h.pwriteF32(10, 0.5)
  const s = Slice.over(h, 0, 14) // the window spans exactly the 14 bytes written
  assert.equal(s.preadI16(0), -12345)
  assert.equal(s.preadU64(2), 999n)
  assert.equal(s.preadF32(10), 0.5)
  // A slice is read-only for typed scalars — no pwrite* counterparts.
  assert.equal(typeof s.pwriteI16, 'undefined')
  assert.equal(typeof s.preadU128, 'function')
})

// -------------------------------------------------------------------------------------
// Element data type (dtype / setDtype / elementCount) + resize + mask filter
// -------------------------------------------------------------------------------------

test('dtype defaults to Unknown; setDtype declares it; elementCount steps by the width', () => {
  const h = new Heap()
  h.pwriteI64Array(0, [1, 2, 3])
  assert.equal(h.dtype().name(), 'unknown') // no declared type
  assert.equal(h.elementCount(), 0) // Unknown -> no element count

  h.setDtype(DataTypeId.I64())
  assert.ok(h.dtype().equals(DataTypeId.I64()))
  assert.equal(h.dtype().asU16(), 8)
  assert.equal(h.elementCount(), 3) // 24 bytes / 8
  // The type is stored in the headers as X-Type-Id.
  assert.equal(h.headers.typeId().name(), 'i64')
  assert.equal(h.headers.typeByteSize(), 8)

  // Unknown clears it.
  h.setDtype(DataTypeId.Unknown())
  assert.equal(h.dtype().name(), 'unknown')
})

test('Headers element-type + name accessors round-trip', () => {
  const meta = new Headers()
  assert.equal(meta.typeId().name(), 'unknown') // default
  assert.equal(meta.typeByteSize(), 0)
  assert.equal(meta.typeBitSize(), 0)
  assert.equal(meta.name(), null) // no X-Name yet

  meta.setTypeId(DataTypeId.F32())
  assert.ok(meta.typeId().equals(DataTypeId.F32()))
  assert.equal(meta.typeByteSize(), 4)
  assert.equal(meta.typeBitSize(), 32)

  // Unknown removes the header.
  meta.setTypeId(DataTypeId.Unknown())
  assert.equal(meta.typeId().name(), 'unknown')

  meta.setName('column-a')
  assert.equal(meta.name(), 'column-a')
})

test('resizeDtype returns a fresh narrowed heap; resizeDtypeInPlace rewrites at the new width', () => {
  const src = new Heap()
  src.pwriteI64Array(0, [1, -2, 3])
  src.setDtype(DataTypeId.I64())

  const narrowed = src.resizeDtype(DataTypeId.I32())
  assert.ok(narrowed instanceof Heap)
  assert.equal(narrowed.byteSize(), 12) // 3 * 4
  assert.equal(src.byteSize(), 24) // source untouched
  assert.deepEqual(narrowed.preadI32Array(0, 3), [1, -2, 3])
  assert.ok(narrowed.dtype().equals(DataTypeId.I32()))

  // In place: rewrites this heap and returns the element count.
  assert.equal(src.resizeDtypeInPlace(DataTypeId.I32()), 3)
  assert.equal(src.byteSize(), 12)
  assert.deepEqual(src.preadI32Array(0, 3), [1, -2, 3])

  // A source with no declared type throws the guided error.
  assert.throws(() => new Heap(Buffer.from('abcd')).resizeDtype(DataTypeId.I32()), /element type/)
})

test('maskFilter keeps elements whose mask bit is set (LSB-first); in-place compacts + truncates', () => {
  const data = new Heap()
  data.pwriteI32Array(0, [10, 20, 30, 40])
  data.setDtype(DataTypeId.I32())

  // 0b0000_1010 -> keep elements 1 and 3.
  const mask = new Heap(Buffer.from([0b0000_1010]))
  const kept = data.maskFilter(mask)
  assert.deepEqual(kept.preadI32Array(0, 2), [20, 40])
  assert.equal(data.byteSize(), 16) // source untouched

  // In place returns the kept count and truncates.
  assert.equal(data.maskFilterInPlace(mask), 2)
  assert.deepEqual(data.preadI32Array(0, 2), [20, 40])
  assert.equal(data.byteSize(), 8)

  // No declared element type -> guided throw.
  assert.throws(() => new Heap(Buffer.from('abcd')).maskFilterInPlace(mask), /element type/)
})

// -------------------------------------------------------------------------------------
// Vectorized aggregations (sum / min / max / mean / std / first / last / countGe)
// -------------------------------------------------------------------------------------

test('i32 aggregations over a whole typed source', () => {
  const h = new Heap()
  h.pwriteI32Array(0, [4, 8, 15, 16, 23, 42])
  assert.equal(h.sumI32(0, 6), 108)
  assert.equal(h.minI32(0, 6), 4)
  assert.equal(h.maxI32(0, 6), 42)
  assert.equal(h.meanI32(0, 6), 18)
  assert.equal(h.firstI32(0, 6), 4)
  assert.equal(h.lastI32(0, 6), 42)
  assert.equal(h.countGeI32(0, 6, 16), 3) // 16, 23, 42
  assert.ok(Math.abs(h.stdI32(0, 6) - 12.315) < 0.01) // sqrt(910/6), population std

  // An empty range: reductions are null, sum/count are 0.
  assert.equal(h.minI32(0, 0), null)
  assert.equal(h.maxI32(0, 0), null)
  assert.equal(h.meanI32(0, 0), null)
  assert.equal(h.stdI32(0, 0), null)
  assert.equal(h.firstI32(0, 0), null)
  assert.equal(h.lastI32(0, 0), null)
  assert.equal(h.sumI32(0, 0), 0)
  assert.equal(h.countGeI32(0, 0, 0), 0)

  // A count past the end throws the guided EOF error.
  assert.throws(() => h.sumI32(0, 7), /unexpected end of data/)
})

test('i64 / u64 sums are BigInt; thresholds cross as BigInt', () => {
  const h = new Heap()
  h.pwriteI64Array(0, [1000, 2000, 3000])
  assert.equal(h.sumI64(0, 3), 6000n) // i128 accumulator -> BigInt
  assert.equal(h.minI64(0, 3), 1000) // a JS number
  assert.equal(h.maxI64(0, 3), 3000)
  assert.equal(h.meanI64(0, 3), 2000)
  assert.equal(h.firstI64(0, 3), 1000)
  assert.equal(h.lastI64(0, 3), 3000)
  assert.equal(h.countGeI64(0, 3, 2000n), 2) // threshold is a BigInt

  const u = new Heap()
  u.pwriteU64Array(0, [10, 20, 30])
  assert.equal(u.sumU64(0, 3), 60n) // BigInt
  assert.equal(u.minU64(0, 3), 10n) // u64 values cross as BigInt
  assert.equal(u.maxU64(0, 3), 30n)
  assert.equal(u.firstU64(0, 3), 10n)
  assert.equal(u.lastU64(0, 3), 30n)
  assert.equal(u.meanU64(0, 3), 20)
  assert.equal(u.countGeU64(0, 3, 20n), 2)
})

test('u32 aggregations', () => {
  const h = new Heap()
  h.pwriteU32Array(0, [5, 10, 15, 20])
  assert.equal(h.sumU32(0, 4), 50)
  assert.equal(h.minU32(0, 4), 5)
  assert.equal(h.maxU32(0, 4), 20)
  assert.equal(h.meanU32(0, 4), 12.5)
  assert.equal(h.firstU32(0, 4), 5)
  assert.equal(h.lastU32(0, 4), 20)
  assert.equal(h.countGeU32(0, 4, 15), 2)
  assert.ok(h.stdU32(0, 4) > 0)
})

test('f32 / f64 aggregations widen to JS numbers; float min/max ignore NaN', () => {
  const f32 = new Heap()
  f32.pwriteF32Array(0, [1.5, 2.5, -0.5, 4.0])
  assert.equal(f32.sumF32(0, 4), 7.5)
  assert.equal(f32.minF32(0, 4), -0.5)
  assert.equal(f32.maxF32(0, 4), 4.0)
  assert.equal(f32.firstF32(0, 4), 1.5)
  assert.equal(f32.lastF32(0, 4), 4.0)
  assert.equal(f32.meanF32(0, 4), 1.875)
  assert.equal(f32.countGeF32(0, 4, 2.0), 2) // 2.5 and 4.0

  const f64 = new Heap()
  f64.pwriteF64Array(0, [10.0, 20.0, 30.0, NaN])
  assert.equal(f64.sumF64(0, 3), 60.0)
  assert.equal(f64.minF64(0, 4), 10.0) // NaN ignored
  assert.equal(f64.maxF64(0, 4), 30.0)
  assert.equal(f64.meanF64(0, 3), 20.0)
  assert.equal(f64.firstF64(0, 3), 10.0)
  assert.equal(f64.lastF64(0, 3), 30.0)
  assert.ok(Math.abs(f64.stdF64(0, 3) - 8.165) < 0.01)
})

// -------------------------------------------------------------------------------------
// Module-level open() — the project's open(), returning the concrete class
// -------------------------------------------------------------------------------------

test('open: a mem:// string/Uri yields a Heap; a Buffer yields a Heap', () => {
  const fromStr = yggdryl.open('mem://heap')
  assert.ok(fromStr instanceof Heap)
  fromStr.pwriteUtf8(0, 'opened')
  assert.equal(fromStr.preadUtf8(0, 6), 'opened')

  const fromUri = yggdryl.open(Uri.parse('mem://heap'))
  assert.ok(fromUri instanceof Heap)

  const fromBytes = yggdryl.open(Buffer.from('bytes'))
  assert.ok(fromBytes instanceof Heap)
  assert.deepEqual(fromBytes.toBytes(), Buffer.from('bytes'))
})

test('open: an unsupported scheme throws the guided error', () => {
  assert.throws(() => yggdryl.open('ftp://example.com/x'), /cannot open the `ftp:\/\/` scheme/)
})

