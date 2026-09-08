'use strict'

// The coupling beside the digest it wraps, through the JavaScript boundary.
// Every row pairs a coupled answer with the plain digest answer for the same
// bytes, so the difference is reading the instant, laying it beside the
// digest, and the value object that crosses back. The final rows fill a
// coupled holder beside a plain one over one batch, IPC copies included.

const { performance } = require('node:perf_hooks')
const arrow = require('apache-arrow')

const { DataType, Field, Scalar, TxHash, TxHasher, txhash, xxhash } = require('yggdryl')

const PAYLOAD = Buffer.from('{"id": 1234567, "venue": "XNAS", "price": "150.2500"}\n'.repeat(20_000))
const INSTANT = 1_700_000_000_000_000n

/** Where a call's fixed cost, the size branches, and the kernel dominate. */
const SIZES = [16, 240, 4096, 64 * 1024]

/** Large enough for per-row work and the IPC boundary to dominate one call. */
const BATCH_ROWS = 4_096

function measure(name, bytes, iterations, operation) {
  for (let index = 0; index < 5; index += 1) operation()
  const started = performance.now()
  for (let index = 0; index < iterations; index += 1) operation()
  const elapsed = performance.now() - started
  const perOperation = elapsed / iterations
  const throughput = bytes / (perOperation / 1_000) / (1000 * 1000 * 1000)
  console.log(
    `${name.padEnd(44)} ${(perOperation * 1_000_000).toFixed(1).padStart(12)} ns ` +
      `${throughput.toFixed(2).padStart(8)} GB/s`,
  )
}

function measureRows(name, rows, iterations, operation) {
  for (let index = 0; index < 5; index += 1) operation()
  const started = performance.now()
  for (let index = 0; index < iterations; index += 1) operation()
  const elapsed = performance.now() - started
  const perOperation = elapsed / iterations
  const throughput = rows / (perOperation / 1_000) / 1_000_000
  console.log(
    `${name.padEnd(44)} ${(perOperation * 1_000).toFixed(1).padStart(12)} us ` +
      `${throughput.toFixed(2).padStart(8)} M row/s`,
  )
}

function main() {
  console.log(`node ${process.version}`)

  for (const size of SIZES) {
    const data = PAYLOAD.subarray(0, size)
    const iterations = size > 4096 ? 20_000 : 200_000
    measure(`xxh3 ${String(size).padStart(9)} B`, size, iterations, () => xxhash.xxh3(data))
    measure(`txh3 ${String(size).padStart(9)} B`, size, iterations, () => txhash.txh3(data, INSTANT))
  }

  const date = new Date('2023-11-14T22:13:20Z')
  const short = PAYLOAD.subarray(0, 240)
  const hasher = new TxHasher('xxh3-64', 'us', 7n)
  const row = Scalar.fromJs([1_234_567, 'XNAS', 150.25, 'AAPL'])
  measure('txh3 240 B (Date instant)', 240, 200_000, () => txhash.txh3(short, date))
  measure('hasher.digest 240 B', 240, 200_000, () => hasher.digest(short, INSTANT))
  measure('hasher.digestScalar (four-column row)', 1, 100_000, () => hasher.digestScalar(row, INSTANT))
  measure('Scalar.digest (four-column row)', 1, 100_000, () => row.digest())
  const value = txhash.txh3(short, INSTANT)
  const spelled = value.toString()
  const bytes = value.bytes()
  measure('value.bytes()', 16, 200_000, () => value.bytes())
  measure('TxHash.fromBytes', 16, 200_000, () => TxHash.fromBytes('us', 'xxh3-64', bytes))
  measure('TxHash.from(string)', 16, 200_000, () => TxHash.from(spelled))
  measure('txhash.unixOf(Date)', 1, 200_000, () => txhash.unixOf(date))

  const events = Array.from({ length: BATCH_ROWS }, (_, index) => INSTANT + BigInt(index))
  const symbols = Array.from({ length: BATCH_ROWS }, (_, index) => (index % 2 === 0 ? 'AAPL' : 'MSFT'))
  const batch = new arrow.Table({
    event: arrow.vectorFromArray(events, new arrow.Int64()),
    symbol: arrow.vectorFromArray(symbols, new arrow.Utf8()),
  }).batches[0]
  const event = new Field('event', 'int64', false)
  const symbol = new Field('symbol', 'utf8', false)
  const plain = new Field('row_digest', 'uint64', false)
  plain.digest.set('role', 'holder')
  const coupled = new Field('key', 'fixed_size_binary[16]', false)
  coupled.digest.set('role', 'holder')
  coupled.digest.set('time', 'event')
  const plainRoot = new Field('row', DataType.fromFields([event, symbol, plain]), false)
  const coupledRoot = new Field('row', DataType.fromFields([event, symbol, coupled]), false)
  const state = new xxhash.Xxh3()
  measureRows('plain holder applyArrowBatch', BATCH_ROWS, 50, () => state.applyArrowBatch(plainRoot, batch))
  measureRows('coupled holder applyArrowBatch', BATCH_ROWS, 50, () => hasher.applyArrowBatch(coupledRoot, batch))
}

main()
