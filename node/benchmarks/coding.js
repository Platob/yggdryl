'use strict'

// A smoke-sized view of the native coding boundary. Increase the iteration
// count explicitly when collecting publication numbers.

const { performance } = require('node:perf_hooks')
const { gzip, zlib, zstd } = require('yggdryl')

const iterations = Number.parseInt(process.env.YGGDRYL_BENCH_ITERATIONS ?? '200', 10)
if (!Number.isSafeInteger(iterations) || iterations <= 0) {
  throw new RangeError('YGGDRYL_BENCH_ITERATIONS must be a positive safe integer')
}

const payload = Buffer.from('{"id":1,"venue":"XNAS"}\n'.repeat(100))
const cases = [
  ['gzip', gzip],
  ['zlib', zlib],
  ['zstd', zstd],
]

for (const [name, coding] of cases) {
  const encoded = coding.dumps(payload)
  for (const [direction, operation] of [
    ['encode', () => coding.dumps(payload)],
    ['decode', () => coding.loads(encoded)],
  ]) {
    const started = performance.now()
    for (let index = 0; index < iterations; index += 1) operation()
    const nanoseconds = ((performance.now() - started) * 1_000_000) / iterations
    console.log(`${name} ${direction}: ${nanoseconds.toFixed(1)} ns/op`)
  }
}
