'use strict'

// Benchmarks for the yggdryl.factory type-inference wrappers.
//
// Measures the per-call cost of inferring a data type from a native value and
// building the matching object across the FFI boundary. No dependencies: run with
// `npm run bench`.

const { factory, scalar } = require('..')

const N = 200_000

function bench(label, fn) {
  fn() // warm-up
  const start = process.hrtime.bigint()
  for (let i = 0; i < N; i++) {
    fn()
  }
  const elapsed = Number(process.hrtime.bigint() - start)
  console.log(`${label.padEnd(36)} ${(elapsed / N).toFixed(1).padStart(9)} ns/op`)
}

const blob = Buffer.from([1, 2, 3, 4])
const row = { id: 42, payload: blob, scores: [1, 2, 3, 4] }
const record = factory.scalar(row)
const half16 = new scalar.Float16Scalar(1.5)

bench('factory.scalar(number)', () => factory.scalar(42))
bench('factory.scalar(float)', () => factory.scalar(1.5))
bench('factory.scalar(bigint)', () => factory.scalar(42n))
bench('factory.scalar(Buffer)', () => factory.scalar(blob))
bench('factory.scalar(string)', () => factory.scalar('hello'))
bench('factory.scalar(Float16Scalar)', () => factory.scalar(half16))
bench('factory.scalar(null)', () => factory.scalar(null))
bench('factory.scalar(array)', () => factory.scalar([1, 2, 3, 4]))
bench('factory.scalar(float array)', () => factory.scalar([1.5, 2.5, 3.5, 4.5]))
bench('factory.scalar(object) record', () => factory.scalar(row))
bench('factory.dtype(number)', () => factory.dtype(42))
bench('factory.dtype(object) struct', () => factory.dtype(row))
bench('factory.field(name, number)', () => factory.field('id', 42))
bench('RecordScalar.toJsValue()', () => record.toJsValue())
