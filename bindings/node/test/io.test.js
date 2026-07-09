'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')

const { ByteBuffer, I32Cursor, I96Cursor, I128Cursor, I256Cursor, I256Slice, Whence } = yggdryl.io
const { I32Buffer } = yggdryl.buffer
const { Gzip } = yggdryl.compression

test('io.ByteBuffer is storage', () => {
  const buf = new ByteBuffer(Buffer.from('data'))
  assert.equal(Number(buf.byteSize()), 4)
  assert.equal(buf.length, 4)
  assert.deepEqual(buf.asBytes(), Buffer.from('data'))
  assert.ok(buf.equals(new ByteBuffer(Buffer.from('data'))))
  assert.ok(ByteBuffer.deserializeBytes(buf.serializeBytes()).equals(buf))
})

test('io.ByteCursor reads/writes advance', () => {
  const cursor = new ByteBuffer().byteCursor()
  assert.equal(Number(cursor.pwriteByteArray(Buffer.from('hello world'))), 11)
  assert.equal(Number(cursor.tell()), 11)
  cursor.seek(0)
  assert.deepEqual(cursor.preadByteArray(5, Whence.Current), Buffer.from('hello'))
  assert.equal(Number(cursor.tell()), 5)
})

test('io.ByteCursor is copy-on-write', () => {
  const buf = new ByteBuffer(Buffer.from('abcdef'))
  const cursor = buf.byteCursor()
  cursor.pwriteByteArray(Buffer.from('XYZ'))
  assert.deepEqual(buf.asBytes(), Buffer.from('abcdef')) // intact
  assert.deepEqual(cursor.asBytes(), Buffer.from('XYZdef'))
})

test('io.ByteCursor seek/position', () => {
  const cursor = new ByteBuffer(Buffer.alloc(10)).byteCursor()
  assert.equal(Number(cursor.seek(3)), 3)
  assert.equal(Number(cursor.seek(2, Whence.Current)), 5)
  assert.equal(Number(cursor.seek(-1, Whence.End)), 9)
  assert.equal(Number(cursor.position()), 9)
})

test('io.ByteCursor typed + endianness', () => {
  const cursor = new ByteBuffer().byteCursor()
  cursor.pwriteI32(-123456)
  cursor.pwriteI32Array([1, 2, 3, -4], Whence.Current)
  cursor.seek(0)
  assert.equal(cursor.preadI32(Whence.Current), -123456)
  assert.deepEqual(cursor.preadI32Array(4, Whence.Current), [1, 2, 3, -4])

  const le = new ByteBuffer().byteCursor()
  le.pwriteU16(0xbeef)
  assert.deepEqual(le.preadByteArray(2, Whence.Start), Buffer.from([0xef, 0xbe]))
})

test('io.ByteCursor size + capacity + BigInt', () => {
  const cursor = new ByteBuffer(Buffer.alloc(10)).byteCursor()
  assert.equal(Number(cursor.byteSize()), 10)
  assert.equal(Number(cursor.size()), 10)
  assert.equal(cursor.largeByteSize(), 10n)
})

test('io transfer between cursors', () => {
  const source = new ByteBuffer(Buffer.from('abcdef')).byteCursor()
  const sink = new ByteBuffer().byteCursor()
  assert.equal(Number(source.preadIo(sink, 3)), 3)
  assert.deepEqual(sink.asBytes(), Buffer.from('abc'))
})

test('io.ByteCursor bit seek is byte-aligned', () => {
  const cursor = new ByteBuffer(Buffer.alloc(10)).byteCursor()
  assert.equal(Number(cursor.bitTell()), 0)
  assert.equal(Number(cursor.bitSeek(16, Whence.Start)), 16) // byte 2
  assert.equal(Number(cursor.tell()), 2)
  assert.throws(() => cursor.bitSeek(17, Whence.Start), /byte-aligned|multiple of 8/)
})

test('io.ByteCursor default accessors', () => {
  const cursor = new ByteBuffer().byteCursor()
  assert.equal(cursor.defaultValue(), 0)
  assert.deepEqual(cursor.defaultByteArray(3), Buffer.from([0, 0, 0]))
})

test('io.I32Cursor counts in i32 units', () => {
  const cursor = new I32Buffer([10, 20, 30, 40]).cursor()
  assert.equal(Number(cursor.tell()), 0)
  assert.equal(cursor.preadOne(Whence.Start), 10)
  assert.equal(Number(cursor.tell()), 1) // one i32 in
  assert.equal(Number(cursor.byteTell()), 4) // four bytes in
  assert.equal(Number(cursor.seek(2, Whence.Start)), 2)
  assert.equal(cursor.preadOne(Whence.Current), 30)
  assert.equal(Number(cursor.seek(-1, Whence.End)), 3)
  assert.equal(cursor.preadOne(Whence.Current), 40)
  // size is the *remaining* count (0 at the end); reset to the start for the total.
  assert.equal(Number(cursor.size()), 0)
  cursor.seek(0, Whence.Start)
  assert.equal(Number(cursor.size()), 4)
})

test('io.I32Cursor write past end fills with default', () => {
  const cursor = I32Cursor.withCapacity(8)
  cursor.pwriteOne(1, Whence.Start)
  cursor.seek(3, Whence.Start) // skip two i32 values
  cursor.pwriteOne(9, Whence.Current)
  cursor.seek(0, Whence.Start)
  assert.equal(Number(cursor.size()), 4) // 4 i32 total, from the start
  assert.deepEqual(cursor.preadArray(4, Whence.Start), [1, 0, 0, 9])
  assert.ok(Number(cursor.capacity()) >= 8)
})

test('io.I32Cursor typed write is copy-on-write', () => {
  const buf = new I32Buffer([1, 2, 3])
  const cursor = buf.cursor()
  cursor.pwriteArray([9, 9], Whence.Start)
  assert.deepEqual(cursor.preadArray(3, Whence.Start), [9, 9, 3])
  assert.deepEqual(buf.toArray(), [1, 2, 3]) // source untouched
})

test('io wide-int cursors round-trip as BigInt', () => {
  const c96 = I96Cursor.withCapacity(3)
  c96.pwriteArray([-(2n ** 95n), 0n, 2n ** 95n - 1n], Whence.Start)
  c96.seek(0)
  assert.deepEqual(c96.preadArray(3, Whence.Start), [-(2n ** 95n), 0n, 2n ** 95n - 1n])
  assert.equal(c96.asBytes().length, 36) // 12 bytes each

  const c128 = I128Cursor.withCapacity(2)
  c128.pwriteOne(-(2n ** 127n), Whence.Start)
  c128.pwriteOne(2n ** 127n - 1n, Whence.Current)
  c128.seek(0)
  assert.deepEqual(c128.preadArray(2, Whence.Start), [-(2n ** 127n), 2n ** 127n - 1n])

  // i256 — values far beyond i128 round-trip.
  const big = 2n ** 200n + 12345n
  const c256 = I256Cursor.withCapacity(2)
  c256.pwriteArray([big, -big], Whence.Start)
  c256.seek(0)
  assert.deepEqual(c256.preadArray(2, Whence.Start), [big, -big])
  assert.equal(c256.asBytes().length, 64) // 32 bytes each
})

test('io wide-int cursor rejects out-of-range BigInt', () => {
  const c96 = I96Cursor.withCapacity(1)
  assert.throws(() => c96.pwriteOne(2n ** 95n, Whence.Start), /out of range/)
})

test('io.ByteSlice is a bounded window', () => {
  const buf = new ByteBuffer(Buffer.from('hello world'))
  const sl = buf.byteSlice(6, 5) // "world"
  assert.equal(Number(sl.sliceOffset()), 6)
  assert.equal(Number(sl.sliceLen()), 5)
  assert.deepEqual(sl.preadByteArray(100), Buffer.from('world')) // clamped
  assert.equal(Number(sl.byteSize()), 0)
  sl.seek(0)
  assert.equal(Number(sl.pwriteByteArray(Buffer.from('EARTHLING'))), 5) // clamped
  assert.deepEqual(sl.asBytes(), Buffer.from('EARTH'))
  assert.deepEqual(buf.asBytes(), Buffer.from('hello world')) // intact
})

test('io typed slice over a buffer', () => {
  const sl = new I32Buffer([10, 20, 30, 40, 50]).slice(1, 3) // [20, 30, 40]
  assert.equal(Number(sl.sliceLen()), 12)
  assert.equal(Number(sl.size()), 3)
  assert.deepEqual(sl.preadArray(100, Whence.Start), [20, 30, 40]) // clamped
  sl.seek(-1, Whence.End)
  assert.equal(sl.preadOne(Whence.Current), 40)
})

test('io wide slice round-trips as BigInt', () => {
  const big = 2n ** 200n + 7n
  const toBytes = (v) => {
    const out = Buffer.alloc(32)
    let x = v < 0n ? (1n << 256n) + v : v
    for (let i = 0; i < 32; i++) {
      out[i] = Number(x & 0xffn)
      x >>= 8n
    }
    return out
  }
  const sl = I256Slice.fromBytes(Buffer.concat([toBytes(big), toBytes(big)]), 0, 64)
  assert.equal(Number(sl.sliceLen()), 64)
  assert.deepEqual(sl.preadArray(2, Whence.Start), [big, big])
})

test('compression.Gzip streams between cursors', () => {
  const gzip = new Gzip(6)
  const original = Buffer.from('stream me '.repeat(500))
  const source = new ByteBuffer(original).byteCursor()
  const packed = new ByteBuffer().byteCursor()
  gzip.compressStream(source, packed)
  assert.ok(Number(packed.byteSize()) < original.length)

  packed.seek(0)
  const restored = new ByteBuffer().byteCursor()
  gzip.decompressStream(packed, restored)
  assert.deepEqual(restored.asBytes(), original)
})
