'use strict'

// Benchmark yggdryl.converter against Node's native Number/String/typed-array cast
// for the same three operations: flexibly parsing decimal strings to i32, rendering
// i32 to strings, and bulk-casting i32 bytes to i64. Build the addon in RELEASE first
// (npm run build) — a debug build is meaningless.
//
// Run with:  node bindings/node/benchmark/converter.bench.js

const { converter } = require('..')

const N = 100_000
const ITERS = 50
const VALUES = Array.from({ length: N }, (_, i) => i)
const STRINGS = VALUES.map(String)
const PARSE_BYTES = STRINGS.reduce((sum, s) => sum + s.length, 0)
const FORMAT_BYTES = PARSE_BYTES
const CAST_SIZE = N * 4
const CAST_DATA = Buffer.from(Int32Array.from(VALUES).buffer.slice(0))

function throughputMbS(nbytes, iters, op) {
  op() // warm up
  const start = process.hrtime.bigint()
  for (let i = 0; i < iters; i++) op()
  const secs = Number(process.hrtime.bigint() - start) / 1e9
  return (nbytes * iters) / secs / (1024 * 1024)
}

function yggParse() {
  for (const s of STRINGS) converter.parse(s, 'i32')
}
function stdParse() {
  for (const s of STRINGS) Number(s)
}
function yggFormat() {
  for (const v of VALUES) converter.format(v, 'i32')
}
function stdFormat() {
  for (const v of VALUES) String(v)
}
function yggCast() {
  converter.cast(CAST_DATA, 'i32', 'i64')
}
function stdCast() {
  // Widen i32 -> i64 via a native typed array.
  BigInt64Array.from(new Int32Array(CAST_DATA.buffer.slice(0)), BigInt)
}

function main() {
  console.log(`yggdryl.converter vs native, ${N} values, ${ITERS} iters:\n`)
  const header = ['op', 'yggdryl', 'native', 'ratio']
    .map((c, i) => c.padStart(i === 0 ? 16 : i < 3 ? 10 : 7))
    .join('  ')
  console.log(header)
  console.log('-'.repeat(header.length))
  const cases = [
    ['parse->i32', PARSE_BYTES, yggParse, stdParse],
    ['format i32', FORMAT_BYTES, yggFormat, stdFormat],
    ['cast i32->i64', CAST_SIZE, yggCast, stdCast],
  ]
  for (const [name, nbytes, yggOp, stdOp] of cases) {
    const ygg = throughputMbS(nbytes, ITERS, yggOp)
    const std = throughputMbS(nbytes, ITERS, stdOp)
    console.log(
      [
        name.padStart(16),
        `${ygg.toFixed(1)}MB`.padStart(10),
        `${std.toFixed(1)}MB`.padStart(10),
        `${(ygg / std).toFixed(2)}x`.padStart(7),
      ].join('  '),
    )
  }
}

main()
