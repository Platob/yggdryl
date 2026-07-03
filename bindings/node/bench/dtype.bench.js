'use strict'

// Benchmarks for the yggdryl.dtype Node wrappers.
//
// Measures the per-call cost of the data-type surface across the FFI boundary
// (dominated by napi call overhead; compare with the Rust-side criterion numbers
// in crates/yggdryl-dtype/benches). No dependencies: run with `npm run bench`.

const { dtype } = require('..')

const N = 200_000

function bench(label, fn) {
  fn() // warm-up
  const start = process.hrtime.bigint()
  for (let i = 0; i < N; i++) {
    fn()
  }
  const elapsed = Number(process.hrtime.bigint() - start)
  console.log(`${label.padEnd(32)} ${(elapsed / N).toFixed(1).padStart(9)} ns/op`)
}

const int64 = new dtype.Int64Type()
const encoded = int64.nativeToBytes(42n)

bench('new Int64Type()', () => new dtype.Int64Type())
bench('nativeToBytes(42n)', () => int64.nativeToBytes(42n))
bench('nativeFromBytes(8B)', () => int64.nativeFromBytes(encoded))
bench('defaultValue()', () => int64.defaultValue())
bench('defaultScalar()', () => int64.defaultScalar())
bench('field("id", false)', () => int64.field('id', false))
bench('scalar(42n)', () => int64.scalar(42n))
bench('new Int64Type().optional()', () => int64.optional())
