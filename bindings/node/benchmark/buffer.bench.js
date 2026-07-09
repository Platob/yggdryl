'use strict'

// Benchmark yggdryl.buffer.I32Buffer against Node's Int32Array / Buffer for the
// same three operations: constructing from a list of values, serialising to bytes,
// and deserialising from bytes. Build the addon in RELEASE first (npm run build) —
// a debug build is meaningless.
//
// Run with:  node bindings/node/benchmark/buffer.bench.js

const { buffer } = require('..')
const { I32Buffer } = buffer

const COUNT = (1 << 20) / 4 // 256 Ki i32 == 1 MiB
const SIZE = COUNT * 4
const ITERS = 200
const VALUES = Array.from({ length: COUNT }, (_, i) => i)
const TYPED = Int32Array.from(VALUES)
const DATA = Buffer.from(TYPED.buffer.slice(0))

function throughputMbS(nbytes, iters, op) {
  op() // warm up
  const start = process.hrtime.bigint()
  for (let i = 0; i < iters; i++) op()
  const secs = Number(process.hrtime.bigint() - start) / 1e9
  return (nbytes * iters) / secs / (1024 * 1024)
}

function main() {
  console.log(`I32Buffer vs Int32Array over ${SIZE >> 10} KiB (${COUNT} i32), ${ITERS} iters:\n`)
  const header = ['op', 'yggdryl', 'Int32Array', 'ratio']
    .map((c, i) => c.padStart(i === 0 ? 12 : i < 3 ? 10 : 7))
    .join('  ')
  console.log(header)
  console.log('-'.repeat(header.length))

  // pre-built inputs so each row measures one pure operation; the native serialize
  // does a real byte copy (`copyBytesFrom`), the fair analogue of `serializeBytes`.
  const yggBuf = new I32Buffer(VALUES)
  const cases = [
    ['construct', () => new I32Buffer(VALUES), () => Int32Array.from(VALUES)],
    ['serialize', () => yggBuf.serializeBytes(), () => Buffer.copyBytesFrom(TYPED)],
    ['deserialize', () => I32Buffer.deserializeBytes(DATA), () => new Int32Array(DATA.buffer.slice(0))],
  ]
  for (const [name, yggOp, stdOp] of cases) {
    const ygg = throughputMbS(SIZE, ITERS, yggOp)
    const std = throughputMbS(SIZE, ITERS, stdOp)
    console.log(
      [
        name.padStart(12),
        `${ygg.toFixed(1)}MB`.padStart(10),
        `${std.toFixed(1)}MB`.padStart(10),
        `${(ygg / std).toFixed(2)}x`.padStart(7),
      ].join('  '),
    )
  }
}

main()
