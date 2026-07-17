'use strict'

// Fast time + memory benchmark for yggdryl.local.LocalIO (runs in ~1-2 s).
//
// Exercises what a caller of the local access point touches: the lazy auto-create first write
// (parents + file + mapping on demand), the self-optimized mapped read/write fast path, the
// SIMD bulk typed arrays that delegate to the mapped backing, the ad-hoc vs mapped read gap,
// and the memory-tree directory read. Time is in Mops/s; the V8 heapUsed delta reports bytes
// per bulk read.
//
// True multi-thread concurrency is a Rust-core story (Node is single-threaded per isolate) —
// see benchmarks/yggdryl-core/io/local/io.md for the shared-mapping / disjoint-file scaling.
//
// Build the addon in RELEASE first (npm run build). Run with --expose-gc for cleaner memory:
//
//   node --expose-gc bindings/node/benchmark/io/local.bench.js

const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { local } = require('../..')
const { LocalIO } = local

const ITERS = 10_000
const VALUES = Array.from({ length: 1024 }, (_, i) => i) // the i32 corpus the bulk rows move

function mopsS(items, iters, op) {
  op() // warm up
  const start = process.hrtime.bigint()
  for (let i = 0; i < iters; i++) op()
  const secs = Number(process.hrtime.bigint() - start) / 1e9
  return (items * iters) / secs / 1_000_000
}

function bytesPer(count, build) {
  build() // warm one-time state
  if (global.gc) global.gc()
  const before = process.memoryUsage().heapUsed
  const kept = build()
  const after = process.memoryUsage().heapUsed
  if (kept.length !== count) throw new Error('build size mismatch')
  return (after - before) / count
}

function main() {
  console.log(`yggdryl.local.LocalIO — time & memory (${ITERS} iters)\n`)

  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-localbench-'))
  const root = new LocalIO(tmp)

  // A persistent self-optimized (mapped) handle for the fast-path rows.
  const hot = root.join('hot.bin')
  hot.pwriteI32Array(0, VALUES) // first write maps it
  if (!hot.isMapped) throw new Error('hot handle should be mapped')

  // A directory memory tree: 16 file blocks of 256 bytes.
  const tree = root.join('tree')
  for (let i = 0; i < 16; i++) {
    const block = tree.join(`b${String(i).padStart(2, '0')}.bin`)
    block.pwriteByteArray(0, Buffer.alloc(256, i))
    block.close()
  }

  let lazyN = 0

  const ops = [
    ['lazy first write (mkdir+create+map)', () => {
      const node = new LocalIO(path.join(tmp, `lazy/d${lazyN++}/note.bin`))
      node.pwriteI64(0, 2 ** 40)
      node.close()
    }, 1],
    ['mapped pwriteI32+preadI32', () => { hot.pwriteI32(64, -1); hot.preadI32(64) }, 2],
    ['bulk pwriteI32Array (1024)', () => hot.pwriteI32Array(0, VALUES), 1024],
    ['bulk preadI32Array (1024)', () => hot.preadI32Array(0, 1024), 1024],
    ['ad-hoc pread 4 KiB (never written)', () => new LocalIO(path.join(tmp, 'hot.bin')).preadByteArray(0, 4096), 1],
    ['tree byteSize + pread (16x256)', () => { tree.byteSize(); tree.preadByteArray(0, 16 * 256) }, 1],
  ]
  console.log('time (Mops/s):')
  for (const [name, op, items] of ops) {
    const iters = items >= 2 ? ITERS : Math.floor(ITERS / 20)
    console.log(`  ${name.padEnd(38)} ${mopsS(items, iters, op).toFixed(3).padStart(9)}`)
  }

  // ---- memory ---------------------------------------------------------------------
  const n = 20_000
  const perRead = bytesPer(n, () => Array.from({ length: n }, () => hot.preadI32Array(0, 256)))
  console.log('\nmemory (V8 heapUsed delta):')
  console.log(`  ${'bytes / preadI32Array(256)'.padEnd(38)} ${perRead.toFixed(1).padStart(9)}`)

  // Self-check: the mapped bulk round-trip is exact, and the tree stitches its blocks.
  if (JSON.stringify(hot.preadI32Array(0, 1024)) !== JSON.stringify(VALUES)) {
    throw new Error('bulk round-trip mismatch')
  }
  if (Number(tree.byteSize()) !== 16 * 256) throw new Error('tree size mismatch')

  hot.close() // release the mapping before cleanup (Windows cannot delete a mapped file)
  fs.rmSync(tmp, { recursive: true, force: true })
}

main()
