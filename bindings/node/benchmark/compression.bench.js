'use strict'

// Compare yggdryl's gzip codec against Node's built-in `zlib`.
//
// Both compress the same corpus at the same level; the script reports MB/s for
// each and the speedup, so the Rust-backed `yggdryl.compression.Gzip` (flate2 /
// miniz_oxide) can be weighed against `zlib` (the C zlib).
//
// Run with:  node bindings/node/benchmark/compression.bench.js

const zlib = require('node:zlib')
const { compression } = require('..')

const CORPUS = Buffer.from('the quick brown fox jumps over the lazy dog. '.repeat(23302)).subarray(0, 1 << 20)
const ITERS = 200
const LEVELS = [1, 6, 9]

function throughputMbS(nbytes, iters, op) {
  op() // warm up
  const start = process.hrtime.bigint()
  for (let i = 0; i < iters; i++) op()
  const secs = Number(process.hrtime.bigint() - start) / 1e9
  return (nbytes * iters) / secs / (1024 * 1024)
}

function main() {
  console.log(`gzip throughput over ${CORPUS.length >> 10} KiB, ${ITERS} iters:\n`)
  const header = ['level', 'op', 'yggdryl', 'zlib', 'speedup']
    .map((c, i) => c.padStart(i < 2 ? 7 : 10))
    .join('  ')
  console.log(header)
  console.log('-'.repeat(header.length))

  for (const level of LEVELS) {
    const ygg = new compression.Gzip(level)
    const packed = ygg.encodeByteArray(CORPUS)

    const cases = [
      ['encode', () => ygg.encodeByteArray(CORPUS), () => zlib.gzipSync(CORPUS, { level })],
      ['decode', () => ygg.decodeByteArray(packed), () => zlib.gunzipSync(packed)],
    ]
    for (const [opName, yggOp, zlibOp] of cases) {
      const yggMb = throughputMbS(CORPUS.length, ITERS, yggOp)
      const zlibMb = throughputMbS(CORPUS.length, ITERS, zlibOp)
      const speedup = yggMb / zlibMb
      console.log(
        [
          String(level).padStart(7),
          opName.padStart(7),
          `${yggMb.toFixed(1)}MB`.padStart(10),
          `${zlibMb.toFixed(1)}MB`.padStart(10),
          `${speedup.toFixed(2)}x`.padStart(10),
        ].join('  '),
      )
    }
  }
}

main()
