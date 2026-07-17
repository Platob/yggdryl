'use strict'

// Fast time + memory benchmark for yggdryl.memory.Heap (runs in ~1-2 s).
//
// Time: the hot Heap ops reported in Mops/s — typed positioned reads (preadI32 / preadI64),
// the bulk preadByteArray, cursor writes, slice windows, and from-Buffer ingest. Memory: the
// V8 heapUsed delta per live Heap ingested from a Buffer and per preadByteArray buffer,
// validating the thin wrapper adds no runaway allocation.
//
// Build the addon in RELEASE first (npm run build) — a debug build is meaningless for the
// timings. Run with `--expose-gc` for cleaner memory numbers:
//
//   node --expose-gc bindings/node/benchmark/memory.bench.js

const { memory } = require('../..')
const { Heap } = memory

const ITERS = 10_000

// A 1 KiB payload of pseudo-typed data (i32s + i64s interleaved) to read back.
const PAYLOAD = Buffer.alloc(1024)
for (let i = 0; i < PAYLOAD.length; i += 4) PAYLOAD.writeInt32LE((i * 2654435761) | 0, i)

// The offsets a typed read sweep visits (word-aligned, in-bounds).
const I32_OFFSETS = Array.from({ length: 64 }, (_, i) => i * 4)
const I64_OFFSETS = Array.from({ length: 32 }, (_, i) => i * 8)

function mopsS(items, iters, op) {
  op() // warm up
  const start = process.hrtime.bigint()
  for (let i = 0; i < iters; i++) op()
  const secs = Number(process.hrtime.bigint() - start) / 1e9
  return (items * iters) / secs / 1_000_000
}

// Peak V8 heapUsed bytes per object while `count` objects built by `build` stay alive.
function bytesPer(count, build) {
  build() // warm one-time state
  if (global.gc) global.gc()
  const before = process.memoryUsage().heapUsed
  const kept = build()
  const after = process.memoryUsage().heapUsed
  // reference `kept` so V8 cannot collect it before the second sample
  if (kept.length !== count) throw new Error('build size mismatch')
  return (after - before) / count
}

function main() {
  console.log(`yggdryl.memory.Heap — time & memory (${ITERS} iters)\n`)

  const src = new Heap(PAYLOAD)

  // ---- time -----------------------------------------------------------------------
  const ops = [
    ['new Heap(Buffer)', () => new Heap(PAYLOAD), 1],
    ['preadI32 sweep', () => { for (const o of I32_OFFSETS) src.preadI32(o) }, I32_OFFSETS.length],
    ['preadI64 sweep', () => { for (const o of I64_OFFSETS) src.preadI64(o) }, I64_OFFSETS.length],
    ['preadByteArray(0,1024)', () => src.preadByteArray(0, 1024), 1],
    ['slice(0,512)', () => src.slice(0, 512), 1],
    ['cursor write 1 KiB', () => { const h = new Heap(); h.write(PAYLOAD) }, 1],
    ['cursor readI32 sweep', () => { src.rewind(); for (let i = 0; i < 256; i++) src.readI32() }, 256],
  ]
  console.log('time (Mops/s):')
  for (const [name, op, items] of ops) {
    console.log(`  ${name.padEnd(26)} ${mopsS(items, ITERS, op).toFixed(2).padStart(7)}`)
  }

  // ---- memory ---------------------------------------------------------------------
  const n = 20_000
  const perHeap = bytesPer(n, () => Array.from({ length: n }, () => new Heap(PAYLOAD)))
  const perRead = bytesPer(n, () => Array.from({ length: n }, () => src.preadByteArray(0, 256)))
  console.log('\nmemory (V8 heapUsed delta):')
  console.log(`  ${'bytes / Heap(1 KiB)'.padEnd(26)} ${perHeap.toFixed(1).padStart(7)}`)
  console.log(`  ${'bytes / preadByteArray'.padEnd(26)} ${perRead.toFixed(1).padStart(7)}`)
}

main()
