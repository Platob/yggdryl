'use strict'

// Benchmarks for the yggdryl.field Node wrappers.
//
// Measures the per-call cost of the field surface across the FFI boundary
// (dominated by napi call overhead; compare with the Rust-side criterion numbers
// in crates/yggdryl-field/benches). No dependencies: run with `npm run bench`.

const { field } = require('..')

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

const column = new field.Int64('id', false)

bench("new Int64('id', false)", () => new field.Int64('id', false))
bench('field.name()', () => column.name())
bench('field.dataType()', () => column.dataType())
bench('field.isNullable()', () => column.isNullable())
