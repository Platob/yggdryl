'use strict'

// Benchmarks for the yggdryl serie wrappers.
//
// Measures the per-call cost of the serie surface across the FFI boundary
// (dominated by napi call overhead plus the element-array conversion; compare
// with the Rust-side criterion numbers in crates/yggdryl-scalar/benches/serie.rs).
// No dependencies: run with `npm run bench`.

const { dtype, scalar } = require('..')

const N = 200_000
// One small serie per call, so the loop stays per-call. The 8-32 bit widths take
// numbers; the 64-bit widths take BigInt.
const ELEMENTS = Array.from({ length: 64 }, (_, index) => index)
const BIG_ELEMENTS = ELEMENTS.map(BigInt)

function bench(label, fn) {
  fn() // warm-up
  const start = process.hrtime.bigint()
  for (let i = 0; i < N; i++) {
    fn()
  }
  const elapsed = Number(process.hrtime.bigint() - start)
  console.log(`${label.padEnd(32)} ${(elapsed / N).toFixed(1).padStart(9)} ns/op`)
}

const numbers = new scalar.Int64Serie(BIG_ELEMENTS)
const narrow = new scalar.Int8Serie(ELEMENTS)
const serieType = new dtype.Int64SerieType()

bench('new Int64Serie(64 BigInts)', () => new scalar.Int64Serie(BIG_ELEMENTS))
bench('new Int8Serie(64 numbers)', () => new scalar.Int8Serie(ELEMENTS))
bench('Int64Serie.null()', () => scalar.Int64Serie.null())
bench('serie.len()', () => numbers.len())
bench('serie.toArray() copy-out', () => numbers.toArray())
bench('serie.valueAt(32)', () => numbers.valueAt(32))
bench('serie.scalarAt(32)', () => numbers.scalarAt(32))
bench('narrow.valueAt(32)', () => narrow.valueAt(32))
bench('SerieType().scalar(64 BigInts)', () => serieType.scalar(BIG_ELEMENTS))
bench('SerieType().nativeToBytes', () => serieType.nativeToBytes(BIG_ELEMENTS))
