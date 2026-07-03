'use strict'

// Benchmarks for the yggdryl.scalar Node wrappers.
//
// Measures the per-call cost of the scalar surface across the FFI boundary
// (dominated by napi call overhead; compare with the Rust-side criterion numbers
// in crates/yggdryl-scalar/benches). No dependencies: run with `npm run bench`.

const { scalar } = require('..')

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

const value = new scalar.Int64Scalar(42n)
const optional = new scalar.OptionalInt64Scalar(42n)

bench('new Int64Scalar(42n)', () => new scalar.Int64Scalar(42n))
bench('Int64Scalar.null()', () => scalar.Int64Scalar.null())
bench('scalar.value()', () => value.value())
bench('scalar.asI64() direct', () => value.asI64())
bench('scalar.asI8() converted', () => value.asI8())
bench('scalar.asF64() checked', () => value.asF64())
bench('new OptionalInt64Scalar(42n)', () => new scalar.OptionalInt64Scalar(42n))
bench('optional.asI64() redirected', () => optional.asI64())
bench('optional.dataType()', () => optional.dataType())
