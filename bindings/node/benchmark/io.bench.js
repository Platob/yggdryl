'use strict'

// Benchmark yggdryl.io.ByteBuffer against Node's Buffer, plus streaming gzip vs
// one-shot zlib. Build the addon in RELEASE first (npm run build) — a debug build
// is meaningless.
//
// Run with:  node bindings/node/benchmark/io.bench.js

const zlib = require('node:zlib')
const { buffer, compression, io } = require('..')
const { ByteBuffer, I64Cursor, I256Cursor, Whence } = io
const { I64Buffer } = buffer

const SIZE = 1 << 20
const ITERS = 200
const CHUNK = 64 * 1024
const DATA = Buffer.alloc(SIZE, 0x78)

function throughputMbS(nbytes, iters, op) {
  op() // warm up
  const start = process.hrtime.bigint()
  for (let i = 0; i < iters; i++) op()
  const secs = Number(process.hrtime.bigint() - start) / 1e9
  return (nbytes * iters) / secs / (1024 * 1024)
}

function main() {
  console.log(`ByteBuffer vs Buffer over ${SIZE >> 10} KiB, ${ITERS} iters:\n`)
  const header = ['op', 'yggdryl', 'Buffer', 'ratio']
    .map((c, i) => c.padStart(i === 0 ? 10 : i < 3 ? 10 : 7))
    .join('  ')
  console.log(header)
  console.log('-'.repeat(header.length))

  const yggWrite = () => {
    const cursor = ByteBuffer.withByteCapacity(SIZE).byteCursor()
    for (let pos = 0; pos < SIZE; pos += CHUNK) {
      cursor.pwriteByteArray(DATA.subarray(pos, Math.min(pos + CHUNK, SIZE)), Whence.Current)
    }
  }
  const bufferWrite = () => {
    const buf = Buffer.allocUnsafe(SIZE)
    for (let pos = 0; pos < SIZE; pos += CHUNK) {
      DATA.copy(buf, pos, pos, Math.min(pos + CHUNK, SIZE))
    }
  }

  const src = new ByteBuffer(DATA)
  const yggRead = () => {
    const cursor = src.byteCursor()
    for (let pos = 0; pos < SIZE; pos += CHUNK) cursor.preadByteArray(CHUNK, Whence.Current)
  }
  const bufferRead = () => {
    for (let pos = 0; pos < SIZE; pos += CHUNK) DATA.subarray(pos, pos + CHUNK)
  }

  for (const [name, yggOp, stdOp] of [
    ['write', yggWrite, bufferWrite],
    ['read', yggRead, bufferRead],
  ]) {
    const ygg = throughputMbS(SIZE, ITERS, yggOp)
    const std = throughputMbS(SIZE, ITERS, stdOp)
    console.log(
      [name.padStart(10), `${ygg.toFixed(1)}MB`.padStart(10), `${std.toFixed(1)}MB`.padStart(10), `${(ygg / std).toFixed(2)}x`.padStart(7)].join('  '),
    )
  }

  // Overhead check: the type-inferring write() should match pwriteByteArray on the
  // Buffer fast path (ratio ~1.0).
  const yggWriteInferred = () => {
    const cursor = ByteBuffer.withByteCapacity(SIZE).byteCursor()
    for (let pos = 0; pos < SIZE; pos += CHUNK) {
      cursor.write(DATA.subarray(pos, Math.min(pos + CHUNK, SIZE)), Whence.Current)
    }
  }
  const inferred = throughputMbS(SIZE, ITERS, yggWriteInferred)
  const explicit = throughputMbS(SIZE, ITERS, yggWrite)
  console.log(
    `  write() inferred ${inferred.toFixed(1)} MB/s   pwriteByteArray ${explicit.toFixed(1)} MB/s   ${(inferred / explicit).toFixed(2)}x (overhead)`,
  )

  console.log('\nTypedCursor<i64> vs BigInt64Array:')
  const count = SIZE / 8
  const values = Array.from({ length: count }, (_, i) => i) // JS numbers (i64 <-> number)
  const valuesBig = BigInt64Array.from(values, BigInt)
  const i64Buf = new I64Buffer(values)

  const yggTypedWrite = () => I64Cursor.withCapacity(count).pwriteArray(values, Whence.Start)
  const bigArrayWrite = () => {
    const out = new BigInt64Array(count)
    out.set(valuesBig)
    return Buffer.from(out.buffer)
  }
  const yggTypedRead = () => i64Buf.cursor().preadArray(count, Whence.Start)
  const raw = Buffer.from(valuesBig.buffer)
  const bigArrayRead = () => new BigInt64Array(raw.buffer, raw.byteOffset, count)

  for (const [name, yggOp, stdOp] of [
    ['write', yggTypedWrite, bigArrayWrite],
    ['read', yggTypedRead, bigArrayRead],
  ]) {
    const ygg = throughputMbS(SIZE, ITERS, yggOp)
    const std = throughputMbS(SIZE, ITERS, stdOp)
    console.log(
      [name.padStart(10), `${ygg.toFixed(1)}MB`.padStart(10), `${std.toFixed(1)}MB`.padStart(10), `${(ygg / std).toFixed(2)}x`.padStart(7)].join('  '),
    )
  }

  console.log('\nI256Cursor vs BigInt <-> bytes (32-byte values):')
  const n256 = SIZE / 32
  const values256 = Array.from({ length: n256 }, (_, i) => BigInt(i))
  const toBytes256 = (v) => {
    const out = Buffer.alloc(32)
    let x = v < 0n ? (1n << 256n) + v : v
    for (let i = 0; i < 32; i++) {
      out[i] = Number(x & 0xffn)
      x >>= 8n
    }
    return out
  }
  const fromBytes256 = (buf, off) => {
    let x = 0n
    for (let i = 31; i >= 0; i--) x = (x << 8n) | BigInt(buf[off + i])
    return x >= 1n << 255n ? x - (1n << 256n) : x
  }
  const ygg256Write = () => I256Cursor.withCapacity(n256).pwriteArray(values256, Whence.Start)
  const jsBig256Write = () => Buffer.concat(values256.map(toBytes256))
  const raw256 = I256Cursor.withCapacity(n256)
  raw256.pwriteArray(values256, Whence.Start)
  const frozen256 = raw256.asBytes()
  const ygg256Read = () => I256Cursor.fromBytes(frozen256).preadArray(n256, Whence.Start)
  const jsBig256Read = () => {
    const out = []
    for (let i = 0; i < frozen256.length; i += 32) out.push(fromBytes256(frozen256, i))
    return out
  }
  for (const [name, yggOp, stdOp] of [
    ['write', ygg256Write, jsBig256Write],
    ['read', ygg256Read, jsBig256Read],
  ]) {
    const ygg = throughputMbS(SIZE, ITERS, yggOp)
    const std = throughputMbS(SIZE, ITERS, stdOp)
    console.log(
      [name.padStart(10), `${ygg.toFixed(1)}MB`.padStart(10), `${std.toFixed(1)}MB`.padStart(10), `${(ygg / std).toFixed(2)}x`.padStart(7)].join('  '),
    )
  }

  console.log('\nByteSlice window read vs Buffer.subarray:')
  const srcBuf = new ByteBuffer(DATA)
  const yggSliceRead = () => {
    const sl = srcBuf.byteSlice(0, SIZE)
    for (let pos = 0; pos < SIZE; pos += CHUNK) sl.preadByteArray(CHUNK, Whence.Current)
  }
  const subarrayRead = () => {
    for (let pos = 0; pos < SIZE; pos += CHUNK) DATA.subarray(pos, pos + CHUNK)
  }
  {
    const ygg = throughputMbS(SIZE, ITERS, yggSliceRead)
    const std = throughputMbS(SIZE, ITERS, subarrayRead)
    console.log(
      ['read'.padStart(10), `${ygg.toFixed(1)}MB`.padStart(10), `${std.toFixed(1)}MB`.padStart(10), `${(ygg / std).toFixed(2)}x`.padStart(7)].join('  '),
    )
  }

  console.log('\ngzip level 6 streaming compression:')
  const gzip = new compression.Gzip(6)
  const yggStream = () =>
    gzip.compressStream(new ByteBuffer(DATA).byteCursor(), ByteBuffer.withByteCapacity(SIZE >> 1).byteCursor())
  const zlibOneShot = () => zlib.gzipSync(DATA, { level: 6 })
  const ygg = throughputMbS(SIZE, ITERS, yggStream)
  const std = throughputMbS(SIZE, ITERS, zlibOneShot)
  console.log(`  yggdryl stream ${ygg.toFixed(1)} MB/s   zlib one-shot ${std.toFixed(1)} MB/s   ${(ygg / std).toFixed(2)}x`)
}

main()
