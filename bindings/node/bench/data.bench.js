'use strict'

// Benchmarks for the yggdryl.data Node wrappers.
//
// Measures the per-call cost of the data-model surface across the FFI boundary
// (dominated by napi call overhead; compare with the Rust-side criterion numbers
// in crates/yggdryl-data/benches). No dependencies: run with `npm run bench`.

const { data } = require('../index.js')

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

const int64 = new data.Int64Type()
const scalar = new data.Int64(42n)
const optional = new data.OptionalInt64(42n)
const encoded = int64.nativeToBytes(42n)

bench('new Int64(42n)', () => new data.Int64(42n))
bench('Int64.null()', () => data.Int64.null())
bench('scalar.value()', () => scalar.value())
bench('scalar.asI64() direct', () => scalar.asI64())
bench('scalar.asI8() converted', () => scalar.asI8())
bench('scalar.asF64() checked', () => scalar.asF64())
bench('new OptionalInt64(42n)', () => new data.OptionalInt64(42n))
bench('optional.asI64() redirected', () => optional.asI64())
bench('optional.dataType()', () => optional.dataType())
bench('new Int64Type().optional()', () => int64.optional())
bench('nativeToBytes(42n)', () => int64.nativeToBytes(42n))
bench('nativeFromBytes(8B)', () => int64.nativeFromBytes(encoded))
bench("new Int64Field('id', false)", () => new data.Int64Field('id', false))
