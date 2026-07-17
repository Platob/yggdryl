'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const nodePath = require('node:path')

const yggdryl = require('../..')
const io = yggdryl.io
const { Headers } = yggdryl.headers
const { LocalEntries, LocalIO, Mmap } = yggdryl.local
const { Whence } = yggdryl.memory
const { Uri } = yggdryl.uri

/** A unique temp directory per test; the test closes its handles, then removes it. */
function tmpDir() {
  return fs.mkdtempSync(nodePath.join(os.tmpdir(), 'yggdryl-node-local-'))
}

/** Removes a test's temp tree (every handle must be closed first — Windows cannot delete
 * a mapped file). */
function rmTree(dir) {
  fs.rmSync(dir, { recursive: true, force: true })
}

let mmapSeq = 0
/** A unique temp-file path per test (deleted by the test after `close()`). */
function tmpFile() {
  mmapSeq += 1
  return nodePath.join(os.tmpdir(), `yggdryl-node-mmap-${process.pid}-${mmapSeq}.bin`)
}

// -------------------------------------------------------------------------------------
// Namespace
// -------------------------------------------------------------------------------------

test('the local namespace exposes LocalIO, LocalEntries, and Mmap', () => {
  assert.equal(typeof LocalIO, 'function')
  assert.equal(typeof LocalEntries, 'function')
  assert.equal(typeof Mmap, 'function')
})

// -------------------------------------------------------------------------------------
// LocalIO — laziness + auto-creating, self-optimizing writes
// -------------------------------------------------------------------------------------

test('LocalIO is lazy: constructing + probing + reading touches nothing on disk', () => {
  const dir = tmpDir()
  const root = new LocalIO(dir)
  const note = root.join('deep/nested/note.txt')

  assert.equal(note.exists(), false)
  assert.equal(note.isMapped, false)
  assert.equal(note.kind, io.IOKind.Missing)
  assert.equal(note.byteSize(), 0)
  assert.deepEqual(note.preadByteArray(0, 16), Buffer.alloc(0)) // missing reads as empty
  assert.equal(note.preadUtf8(0, 5), '')

  // Probing created nothing — the ancestry is still absent.
  assert.ok(!fs.existsSync(nodePath.join(dir, 'deep')))
  rmTree(dir)
})

test('the first write auto-creates parents + the file and keeps the mapped backing', () => {
  const dir = tmpDir()
  const root = new LocalIO(dir)
  const note = root.join('deep/nested/dirs/note.txt')

  assert.equal(note.pwriteUtf8(0, 'hello'), 5)
  assert.ok(note.isFile())
  assert.equal(note.isMapped, true) // self-optimized: later access runs at memory speed
  assert.ok(root.join('deep/nested/dirs').isDir())
  assert.equal(note.preadUtf8(0, 5), 'hello')

  // Typed + bit + bulk + repeat + capacity all work through the same handle.
  note.pwriteI32(8, -7)
  assert.equal(note.preadI32(8), -7)
  note.pwriteBit(111, true) // byte 13, bit 7
  assert.equal(note.preadBit(111), true)
  assert.throws(() => note.preadBit(-1), /invalid bit offset -1/)
  note.pwriteI64Array(16, [1, 2, 3])
  assert.deepEqual(note.preadI64Array(16, 3), [1, 2, 3])
  assert.throws(() => note.preadI32Array(0, 2_000_000_000), /unexpected end of data/)
  note.pwriteByteRepeat(40, 0x77, 300)
  assert.equal(note.preadByte(339), 0x77)
  note.tryReserve(4096)
  assert.ok(note.capacity() >= 4096)
  assert.ok(note.spareCapacity() >= 0)
  note.flush() // persists the mapped bytes — must not throw

  note.close()
  rmTree(dir)
})

test('close() releases the mapping; the handle stays usable, back to lazy', () => {
  const dir = tmpDir()
  const note = new LocalIO(nodePath.join(dir, 'n.bin'))
  note.pwriteUtf8(0, 'hello')
  assert.equal(note.isMapped, true)

  note.close()
  assert.equal(note.isMapped, false)
  assert.equal(note.preadUtf8(0, 5), 'hello') // the ad-hoc read path serves the bytes
  assert.equal(fs.statSync(note.path).size, 5) // truncated to the logical length
  note.close() // idempotent

  // Still writable after close — the next write re-maps.
  note.pwriteByte(5, 0x21) // '!'
  assert.equal(note.isMapped, true)
  assert.equal(note.preadUtf8(0, 6), 'hello!')

  note.close()
  rmTree(dir)
})

test('copy() is a fresh lazy handle: equals by path but not mapped', () => {
  const dir = tmpDir()
  const a = new LocalIO(nodePath.join(dir, 'x.bin'))
  a.pwriteByte(0, 7)
  assert.equal(a.isMapped, true)

  const b = a.copy()
  assert.ok(a.equals(b)) // same path
  assert.equal(b.isMapped, false) // but its own lazy state

  a.close()
  assert.equal(b.preadByte(0), 7)
  rmTree(dir)
})

test('constructor generic dispatch: a string path and a uri.Uri address the same node', () => {
  const dir = tmpDir()
  const file = nodePath.join(dir, 'f.txt')

  const byPath = new LocalIO(file) // string → from_path
  const byUri = new LocalIO(Uri.fromPath(file)) // Uri → from_uri
  assert.ok(byPath.equals(byUri))

  byPath.pwriteUtf8(0, 'shared')
  byPath.close()
  assert.equal(byUri.preadUtf8(0, 6), 'shared')

  // The guided core errors pass through unchanged.
  assert.throws(() => new LocalIO(Uri.parse('http://host/x')), /unsupported scheme/)
  assert.throws(
    () => new LocalIO(Uri.parse('http://host/x')),
    /file:\/\/ URL or a plain path URI/
  )
  assert.throws(() => new LocalIO(Uri.parse('file://localhost')), /empty path; give it a file path/)
  rmTree(dir)
})

// -------------------------------------------------------------------------------------
// LocalIO — IOBase predicates
// -------------------------------------------------------------------------------------

test('isFile / isDir / exists derive from kind on file, directory, and missing nodes', () => {
  const dir = tmpDir()
  const root = new LocalIO(dir)

  const missing = root.join('nothing.bin')
  assert.equal(missing.kind, io.IOKind.Missing)
  assert.ok(!missing.isFile() && !missing.isDir() && !missing.exists())

  const file = root.join('a.bin')
  file.pwriteByte(0, 1)
  assert.equal(file.kind, io.IOKind.File)
  assert.ok(file.isFile() && !file.isDir() && file.exists())

  const d = root.join('d')
  d.mkdir()
  assert.equal(d.kind, io.IOKind.Directory)
  assert.ok(d.isDir() && !d.isFile() && d.exists())

  file.close()
  rmTree(dir)
})

// -------------------------------------------------------------------------------------
// LocalIO — checked reservations + uri round-trips (core-behavior mirrors)
// -------------------------------------------------------------------------------------

test('a read-only LocalIO refuses tryReserve with the guided fix and touches nothing', () => {
  const dir = tmpDir()
  const f = new LocalIO(nodePath.join(dir, 'ro.bin'))
  f.setMode(io.IOMode.Read)

  assert.throws(() => f.tryReserve(64), /read-only/)
  assert.throws(() => f.tryReserve(64), /set_mode/)
  assert.throws(() => f.tryReserveExact(64), /read-only/)
  assert.throws(() => f.tryReserveExact(64), /set_mode/)
  assert.ok(!f.exists()) // the refusal created nothing on disk

  rmTree(dir)
})

test('reserveExact on a fresh handle materializes real capacity', () => {
  const dir = tmpDir()
  const f = new LocalIO(nodePath.join(dir, 'cap.bin'))

  f.reserveExact(4096)
  assert.ok(f.isMapped)
  assert.ok(f.capacity() >= 4096)

  f.close()
  rmTree(dir)
})

test('a path containing a space round-trips through the percent-encoded uri', () => {
  const dir = tmpDir()
  const f = new LocalIO(nodePath.join(dir, 'with space.txt'))
  f.pwriteUtf8(0, 'spaced')

  const uri = f.uri
  assert.ok(uri.toString().includes('%20')) // the space is percent-encoded in the URI
  const back = new LocalIO(uri) // …and percent-decoded on the way back
  assert.ok(back.equals(f))

  f.close()
  assert.equal(back.preadUtf8(0, 6), 'spaced')
  rmTree(dir)
})

// -------------------------------------------------------------------------------------
// LocalIO — navigation + streamed discovery
// -------------------------------------------------------------------------------------

test('navigation: name / parent() / join are pure path math; uri ends with the name', () => {
  const dir = tmpDir()
  const node = new LocalIO(dir).join('a/b/c.txt')
  assert.equal(node.name, 'c.txt')

  const parent = node.parent()
  assert.ok(parent instanceof LocalIO)
  assert.equal(parent.name, 'b')
  assert.equal(parent.parent().name, 'a')
  assert.equal(parent.join('d/e.bin').name, 'e.bin')

  assert.ok(node.uri instanceof Uri)
  assert.ok(node.uri.toString().endsWith('c.txt'))

  // A filesystem root has no parent — the one justified null.
  assert.equal(new LocalIO('/').parent(), null)
  rmTree(dir)
})

test('ls() streams direct children; ls(true) streams the subtree; children() collects', () => {
  const dir = tmpDir()
  const root = new LocalIO(dir)
  for (const [rel, text] of [
    ['one.txt', '1'],
    ['sub/two.txt', '2'],
    ['sub/deeper/three.txt', '3'],
  ]) {
    const node = root.join(rel)
    node.pwriteUtf8(0, text)
    node.close() // release each mapping so the tree can be removed
  }

  // ls() returns a streaming iterable — never a pre-collected array.
  const entries = root.ls()
  assert.ok(entries instanceof LocalEntries)
  assert.ok(!Array.isArray(entries))
  assert.ok(Symbol.iterator in entries)
  assert.equal(typeof entries[Symbol.iterator], 'function')
  assert.equal(entries.toString(), 'LocalEntries(<children>)')
  const iterator = entries[Symbol.iterator]()
  assert.equal(typeof iterator.next, 'function')

  // Spread and for..of both drive the stream; entries are lazy LocalIO handles.
  const direct = [...entries]
  assert.ok(direct[0] instanceof LocalIO)
  assert.deepEqual(direct.map((e) => e.name).sort(), ['one.txt', 'sub'])
  assert.equal(iterator.next().done, true) // one pass: the stream is now exhausted
  const names = []
  for (const entry of root.ls(false)) names.push(entry.name)
  assert.deepEqual(names.sort(), ['one.txt', 'sub'])
  assert.equal(root.children().length, 2) // the collected convenience stays an array

  const walk = root.ls(true)
  assert.equal(walk.toString(), 'LocalEntries(<recursive walk>)')
  const all = [...walk].map((e) => e.name).sort()
  assert.deepEqual(all, ['deeper', 'one.txt', 'sub', 'three.txt', 'two.txt'])

  // A file (and a missing node) streams nothing.
  assert.deepEqual([...root.join('one.txt').ls()], [])
  assert.deepEqual([...root.join('ghost').ls(true)], [])
  assert.deepEqual(root.join('ghost').children(), [])
  rmTree(dir)
})

// -------------------------------------------------------------------------------------
// LocalIO — folders, CRUD, and guided refusals
// -------------------------------------------------------------------------------------

test('mkdir creates the tree; a directory refuses a byte stream with the guided fix', () => {
  const dir = tmpDir()
  const d = new LocalIO(dir).join('a/b/c')
  d.mkdir() // mkdir -p
  assert.ok(d.isDir())

  assert.throws(() => d.pwriteByte(0, 1), /the node is a directory; join_str a file name/)
  assert.equal(d.pwriteByteArray(0, Buffer.from('x')), 0) // the primitive writes nothing
  assert.deepEqual(d.preadByteArray(0, 8), Buffer.alloc(0)) // reads on a directory are empty
  rmTree(dir)
})

test('rm / rmfile / rmdir: guided mismatch errors, idempotent on missing', () => {
  const dir = tmpDir()
  const root = new LocalIO(dir)
  const f = root.join('f.txt')
  f.pwriteUtf8(0, 'x')
  f.close() // release the mapping so Windows can delete
  const d = root.join('d')
  d.mkdir()

  assert.throws(() => d.rmfile(), /use rmdir/)
  assert.throws(() => f.rmdir(), /use rmfile/)

  f.rmfile()
  assert.ok(!f.exists())
  f.rmfile() // idempotent on missing
  d.rmdir()
  assert.ok(!d.exists())
  d.rmdir() // idempotent on missing

  // rm removes whatever exists — a file or a whole tree.
  const g = root.join('g.txt')
  g.pwriteUtf8(0, 'y')
  g.close()
  const i = root.join('h/i.txt')
  i.pwriteUtf8(0, 'z')
  i.close()
  root.join('g.txt').rm()
  root.join('h').rm()
  assert.equal(root.children().length, 0)
  rmTree(dir)
})

// -------------------------------------------------------------------------------------
// LocalIO — cursor stream + ad-hoc reads
// -------------------------------------------------------------------------------------

test('the built-in cursor stream works over a LocalIO file; fresh handles read ad hoc', () => {
  const dir = tmpDir()
  const f = new LocalIO(nodePath.join(dir, 's.bin'))

  assert.equal(f.write(Buffer.from('hello ')), 6)
  f.writeByte(0x7f)
  f.writeI32(-7)
  f.writeI64(2 ** 40)
  assert.equal(f.writeUtf8('wörld'), 6)
  assert.equal(f.position, 6 + 1 + 4 + 8 + 6)

  f.rewind()
  assert.deepEqual(f.read(6), Buffer.from('hello '))
  assert.equal(f.readByte(), 0x7f)
  assert.equal(f.readI32(), -7)
  assert.equal(f.readI64(), 2 ** 40)
  assert.equal(f.readUtf8(6), 'wörld')

  assert.equal(f.seek(Whence.End, -6), f.byteSize() - 6)
  assert.deepEqual(f.readToEnd(), Buffer.from('wörld'))
  assert.throws(() => f.seek(Whence.Start, -1), /invalid seek/)
  f.setPosition(2)
  assert.equal(f.position, 2)

  f.close()
  // A fresh, never-written handle streams the same bytes ad hoc — no mapping.
  const fresh = new LocalIO(nodePath.join(dir, 's.bin'))
  assert.equal(fresh.readUtf8(6), 'hello ')
  assert.equal(fresh.isMapped, false)
  rmTree(dir)
})

// -------------------------------------------------------------------------------------
// LocalIO — metadata + live-handle surface
// -------------------------------------------------------------------------------------

test('metadata: headers copy/setHeaders, the mode gate, path, and toString', () => {
  const dir = tmpDir()
  const f = new LocalIO(nodePath.join(dir, 'm.bin'))
  assert.equal(f.path, nodePath.join(dir, 'm.bin'))

  // headers: the getter returns a copy; setHeaders writes back.
  assert.ok(f.headers instanceof Headers)
  assert.ok(f.headers.isEmpty())
  const headers = f.headers
  headers.insert('Content-Type', 'application/octet-stream')
  assert.ok(f.headers.isEmpty()) // the getter was a copy
  f.setHeaders(headers)
  assert.equal(f.headers.contentType(), 'application/octet-stream')

  // mode: ReadWrite by default; a Read handle refuses writes with the guided fix.
  assert.equal(f.mode, io.IOMode.ReadWrite)
  f.setMode(io.IOMode.Read)
  assert.equal(f.mode, io.IOMode.Read)
  assert.throws(() => f.pwriteByte(0, 1), /read-only/)
  assert.throws(() => f.pwriteByte(0, 1), /set_mode\(ReadWrite\)/)
  assert.equal(f.pwriteByteArray(0, Buffer.from('x')), 0) // the primitive writes nothing
  f.setMode(io.IOMode.ReadWrite)
  f.pwriteUtf8(0, 'ok')

  // toString names the path and the size; uri round-trips into the constructor.
  assert.ok(f.toString().startsWith('LocalIO('))
  assert.ok(f.toString().includes('2 bytes'))
  assert.ok(new LocalIO(f.uri).equals(f))

  // A live handle has no value surface beyond equals/copy (Mmap precedent).
  assert.equal(f.hashCode, undefined)
  assert.equal(f.serializeBytes, undefined)

  f.close()
  rmTree(dir)
})

// -------------------------------------------------------------------------------------
// Mmap — the memory-mapped file source
// -------------------------------------------------------------------------------------

test('Mmap.create: typed, bulk, repeat, and utf8 round-trips over a file', () => {
  const file = tmpFile()
  const m = Mmap.create(file)
  assert.ok(m instanceof Mmap)
  assert.ok(m.isEmpty())
  assert.equal(m.byteSize(), 0)

  // Typed positioned accessors (little-endian, growing + zero-filling like Heap).
  m.pwriteI32(0, -42)
  m.pwriteI64(4, 2 ** 40) // below 2^53
  m.pwriteByte(12, 0xab)
  assert.equal(m.preadI32(0), -42)
  assert.equal(m.preadI64(4), 2 ** 40)
  assert.equal(m.preadByte(12), 0xab)
  assert.equal(m.bitSize(), m.byteSize() * 8)

  // Bit accessors are LSB-first with i64 offsets; negatives throw the guided error.
  m.pwriteBit(111, true) // byte 13, bit 7
  assert.equal(m.preadBit(111), true)
  assert.equal(m.preadBit(110), false)
  assert.throws(() => m.preadBit(-1), /invalid bit offset -1/)

  // Byte-array primitives: short reads near the end, growing writes.
  assert.equal(m.pwriteByteArray(14, Buffer.from('xy')), 2)
  assert.deepEqual(m.preadByteArray(14, 100), Buffer.from('xy')) // clamped at the end
  assert.deepEqual(m.preadByteArray(999, 4), Buffer.alloc(0)) // past the end

  // Bulk typed arrays (300 elements crosses the 256-element staging chunk).
  const i32s = Array.from({ length: 300 }, (_, i) => (i % 2 ? -1 : 1) * i * 1000)
  m.pwriteI32Array(16, i32s)
  assert.deepEqual(m.preadI32Array(16, 300), i32s)
  const i64s = Array.from({ length: 10 }, (_, i) => i * 2 ** 40 + i)
  m.pwriteI64Array(1216, i64s)
  assert.deepEqual(m.preadI64Array(1216, 10), i64s)
  assert.throws(() => m.preadI32Array(0, 2_000_000_000), /unexpected end of data/) // hostile
  assert.throws(() => m.preadI64Array(0, 2_000_000_000), /unexpected end of data/) // count guard

  // Repeated-value fills.
  m.pwriteByteRepeat(1296, 0x77, 300)
  assert.equal(m.preadByte(1595), 0x77)
  m.pwriteI32Repeat(1596, -1, 300)
  assert.ok(m.preadI32Array(1596, 300).every((v) => v === -1))
  m.pwriteI64Repeat(2796, 2 ** 40 + 7, 10)
  assert.equal(m.preadI64(2868), 2 ** 40 + 7)

  // Positioned UTF-8 (byte counts, clamped decode, cut-char throws).
  assert.equal(m.pwriteUtf8(2876, 'héllo'), 6)
  assert.equal(m.preadUtf8(2876, 6), 'héllo')
  assert.throws(() => m.preadUtf8(2877, 1), /invalid UTF-8/) // cuts é in half

  m.close()
  fs.rmSync(file)
})

test('Mmap cursor stream: read/write/typed/seek/readToEnd over a file', () => {
  const file = tmpFile()
  const m = Mmap.create(file)

  assert.equal(m.write(Buffer.from('hello ')), 6)
  m.writeByte(0x7f)
  m.writeI32(-7)
  m.writeI64(2 ** 40)
  assert.equal(m.writeUtf8('wörld'), 6)
  assert.equal(m.position, 6 + 1 + 4 + 8 + 6)

  m.rewind()
  assert.equal(m.position, 0)
  assert.deepEqual(m.read(6), Buffer.from('hello '))
  assert.equal(m.readByte(), 0x7f)
  assert.equal(m.readI32(), -7)
  assert.equal(m.readI64(), 2 ** 40)
  assert.equal(m.readUtf8(6), 'wörld')

  // Seek from every anchor; readToEnd drains to the end.
  assert.equal(m.seek(Whence.Start, 19), 19)
  assert.equal(m.seek(Whence.Current, -1), 18)
  assert.equal(m.seek(Whence.End, -6), m.byteSize() - 6)
  assert.deepEqual(m.readToEnd(), Buffer.from('wörld'))
  assert.equal(m.position, m.byteSize())
  assert.deepEqual(m.read(5), Buffer.alloc(0)) // at the end
  assert.throws(() => m.seek(Whence.Start, -1), /invalid seek/)

  // setPosition past the end zero-fills on the next write, like Heap.
  const size = m.byteSize()
  m.setPosition(size + 2)
  m.write(Buffer.from('Z'))
  assert.equal(m.preadByte(size), 0)
  assert.equal(m.preadByte(size + 2), 0x5a)

  m.close()
  fs.rmSync(file)
})

test('Mmap.open generic dispatch: a string path and a uri.Uri open the same file', () => {
  const file = tmpFile()
  fs.writeFileSync(file, 'hello world')

  const byPath = Mmap.open(file) // string → open_path
  assert.equal(byPath.byteSize(), 11)
  assert.equal(byPath.preadUtf8(0, 5), 'hello')
  byPath.close()

  const byUri = Mmap.open(Uri.fromPath(file)) // Uri → open_uri
  assert.equal(byUri.byteSize(), 11)
  assert.equal(byUri.preadUtf8(6, 5), 'world')
  byUri.close()

  fs.rmSync(file)
})

test('Mmap persistence: close() truncates the padding; reopen sees the exact bytes', () => {
  const file = tmpFile()
  const m = Mmap.create(file)
  m.writeUtf8('hello mapped world') // 18 bytes; capacity grows past it
  assert.ok(m.capacity() >= 18)
  m.close() // unmap + truncate the capacity padding

  assert.equal(fs.statSync(file).size, 18) // the on-disk file is exactly the logical length

  const back = Mmap.open(file)
  assert.equal(back.byteSize(), 18)
  assert.equal(back.preadUtf8(0, 18), 'hello mapped world')
  back.close()
  fs.rmSync(file)
})

test('Mmap.open on a missing file throws the guided error naming the path', () => {
  const file = tmpFile()
  assert.throws(() => Mmap.open(file), /check that the path exists/)
  assert.throws(() => Mmap.open(file), /cannot open/)
  assert.throws(() => Mmap.openReadonly(file), /check that the path exists/)
})

test('Mmap.openReadonly: reads work, writes are inert, tryReserve throws, mode is Read', () => {
  const file = tmpFile()
  fs.writeFileSync(file, 'readonly data')

  const m = Mmap.openReadonly(file)
  assert.equal(m.mode, io.IOMode.Read)
  assert.equal(m.preadUtf8(0, 8), 'readonly')
  assert.deepEqual(m.preadByteArray(9, 4), Buffer.from('data'))

  // The write primitives write nothing (count 0); the full writes name the fix.
  assert.equal(m.pwriteByteArray(0, Buffer.from('X')), 0)
  assert.equal(m.pwriteUtf8(0, 'X'), 0)
  assert.throws(() => m.pwriteByte(0, 1), /read-only/)
  assert.throws(() => m.pwriteI32(0, 1), /open_uri \/ create_uri/) // the guided fix
  assert.throws(() => m.tryReserve(1024), /read-only/)
  assert.deepEqual(m.preadByteArray(0, 8), Buffer.from('readonly')) // bytes untouched

  m.close()
  fs.rmSync(file)
})

test('Mmap capacity family over a file: reserve/ensure/spare/shrink', () => {
  const file = tmpFile()
  const m = Mmap.create(file)
  m.writeUtf8('abc')

  m.tryReserve(1024)
  assert.ok(m.capacity() >= 1027)
  assert.equal(m.spareCapacity(), m.capacity() - 3)
  m.tryReserveExact(2048)
  m.reserve(100)
  m.reserveExact(100)

  m.ensureCapacity(8192)
  assert.ok(m.capacity() >= 8192)
  const cap = m.capacity()
  m.tryEnsureCapacity(16) // already satisfied — a no-op
  assert.equal(m.capacity(), cap)

  // shrink releases the padding down to the logical length; the bytes survive.
  m.shrinkTo(64)
  m.shrinkToFit()
  assert.equal(m.capacity(), 3)
  assert.equal(m.preadUtf8(0, 3), 'abc')

  m.close()
  fs.rmSync(file)
})

test('Mmap auto-grows on appends (amortized) and zero-fills write gaps', () => {
  const file = tmpFile()
  const m = Mmap.create(file)

  const chunk = Buffer.alloc(1000, 0x61)
  for (let i = 0; i < 10; i += 1) assert.equal(m.write(chunk), 1000)
  assert.equal(m.byteSize(), 10000)
  assert.ok(m.capacity() >= 10000)
  assert.equal(m.preadByte(9999), 0x61)

  // A positioned write past the end grows and zero-fills the gap.
  m.pwriteByte(10005, 0xff)
  assert.equal(m.byteSize(), 10006)
  assert.equal(m.preadByte(10002), 0)
  assert.equal(m.preadByte(10005), 0xff)

  m.close()
  fs.rmSync(file)
})

test('Mmap metadata: kind is File, uri round-trips the path, headers/setMode', () => {
  const file = tmpFile()
  const m = Mmap.create(file)

  assert.equal(m.kind, io.IOKind.File)
  assert.ok(m.isFile() && !m.isDir() && m.exists()) // a live mapping is a live file
  assert.equal(m.path, file)

  // The uri is the file path as a POSIX-slash URI — and reopens the same mapping.
  assert.ok(m.uri instanceof Uri)
  assert.equal(m.uri.path, file.replaceAll('\\', '/'))
  assert.ok(m.uri.equals(Uri.fromPath(file)))

  // headers: the getter returns a copy; setHeaders writes back (no withHeaders — no copy).
  assert.ok(m.headers instanceof Headers)
  assert.ok(m.headers.isEmpty())
  const headers = m.headers
  headers.insert('Content-Type', 'application/octet-stream')
  assert.ok(m.headers.isEmpty()) // the getter was a copy
  m.setHeaders(headers)
  assert.equal(m.headers.contentType(), 'application/octet-stream')
  assert.equal(m.withHeaders, undefined) // a live mapping cannot be copied

  // mode: ReadWrite from create; setMode relabels in place (no withMode — no copy).
  assert.equal(m.mode, io.IOMode.ReadWrite)
  m.setMode(io.IOMode.Read)
  assert.equal(m.mode, io.IOMode.Read)
  assert.equal(m.withMode, undefined)

  // DESIGN mirror: no value surface and no Heap-monomorphic views on a live OS resource.
  assert.equal(m.equals, undefined)
  assert.equal(m.copy, undefined)
  assert.equal(m.serializeBytes, undefined)
  assert.equal(m.cursor, undefined)
  assert.equal(m.window, undefined)

  m.close()
  fs.rmSync(file)
})

test('Mmap.flush persists the mapped bytes to disk without closing', () => {
  const file = tmpFile()
  const m = Mmap.create(file)
  m.writeUtf8('flushed')
  m.flush() // msync/FlushViewOfFile + fsync — must not throw

  // The bytes are on disk while the mapping is still open (the file keeps its
  // capacity padding until close(), so compare only the logical length).
  const onDisk = fs.readFileSync(file).subarray(0, m.byteSize())
  assert.deepEqual(onDisk, Buffer.from('flushed'))

  m.close()
  fs.rmSync(file)
})

test('Mmap.close is deterministic and idempotent; use-after-close throws the guided error', () => {
  const file = tmpFile()
  const m = Mmap.create(file)
  m.writeUtf8('bye')

  // toString names the path and logical size while open.
  assert.ok(m.toString().includes(file))
  assert.ok(m.toString().includes('3 bytes'))

  m.close()
  m.close() // idempotent

  // Every method throws the guided closed error; toString stays total for coercion.
  assert.equal(m.closed, true)
  assert.throws(() => m.byteSize(), /the mapping is closed; reopen it/)
  assert.throws(() => m.isFile(), /closed/)
  assert.throws(() => m.exists(), /closed/)
  assert.throws(() => m.preadByte(0), /closed/)
  assert.throws(() => m.pwriteUtf8(0, 'x'), /closed/)
  assert.throws(() => m.flush(), /closed/)
  assert.throws(() => m.position, /closed/)
  assert.equal(m.toString(), 'Mmap(closed)')

  // Closed means unmapped — on Windows a mapped file cannot be deleted, so this
  // succeeding is itself the proof the mapping was released deterministically.
  fs.rmSync(file)
  assert.ok(!fs.existsSync(file))
})
