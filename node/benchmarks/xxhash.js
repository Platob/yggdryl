'use strict'

// The digest boundary: every algorithm at the sizes where a call's fixed cost,
// the size branches, and the streaming kernel each dominate in turn, plus the
// conversion cost of each byte shape. The conversion is deliberately visible
// rather than averaged away - a `Buffer` is borrowed, a plain `ArrayBuffer` is
// copied into one before it crosses, and a `string` is encoded. The final rows
// isolate Arrow holder insertion, conditional filling, and force; they include
// the JavaScript boundary's required IPC copies.

const { performance } = require('node:perf_hooks')
const arrow = require('apache-arrow')

const { DataType, Field, Scalar, xxhash } = require('yggdryl')

const PAYLOAD = Buffer.from('{"id": 1234567, "venue": "XNAS", "price": "150.2500"}\n'.repeat(20_000))

/** The sizes a digest's cost changes shape at. */
const SIZES = [1, 4, 16, 64, 128, 240, 1024, 64 * 1024, 1024 * 1024]

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

  // One-shot, per size: where the dispatch cost stops mattering.
  for (const size of SIZES) {
    const data = Buffer.alloc(size)
    PAYLOAD.copy(data, 0, 0, Math.min(size, PAYLOAD.length))
    const iterations = size > 64 * 1024 ? 2_000 : 200_000
    measure(`xxh3 ${String(size).padStart(9)} B`, size, iterations, () => xxhash.xxh3(data))
  }

  // Every algorithm at one size, so the widths compare directly.
  const size = PAYLOAD.length
  for (const [name, native] of [
    ['xxh32', xxhash.xxh32],
    ['xxh64', xxhash.xxh64],
    ['xxh3', xxhash.xxh3],
    ['xxh128', xxhash.xxh128],
  ]) {
    measure(`${name} payload`, size, 500, () => native(PAYLOAD))
  }

  // The conversion cost, made visible.
  const array = new Uint8Array(PAYLOAD)
  const buffer = array.buffer.slice(0)
  const text = PAYLOAD.toString()
  measure('xxh3 payload (Uint8Array)', size, 500, () => xxhash.xxh3(array))
  measure('xxh3 payload (ArrayBuffer)', size, 500, () => xxhash.xxh3(buffer))
  measure('xxh3 payload (string)', size, 500, () => xxhash.xxh3(text))

  // Streaming against one-shot, and the digest wrapper against the bare bigint.
  measure('xxh3 payload (streamed 64 KiB)', size, 500, () => {
    const state = new xxhash.Xxh3()
    for (let index = 0; index < PAYLOAD.length; index += 64 * 1024) {
      state.writeBytes(PAYLOAD.subarray(index, index + 64 * 1024))
    }
    return state.asDigest().value()
  })
  measure('digest payload (Digest wrapper)', size, 500, () => xxhash.digest(PAYLOAD, 'xxh3-64'))

  // The value feed: a leaf, a wide record, and the hash a table already reads.
  const leaf = Scalar.fromJs('AAPL')
  const record = Scalar.fromJs(
    Object.fromEntries(Array.from({ length: 64 }, (_, index) => [`column_${index}`, BigInt(index)])),
  )
  measure('scalar leaf digest', 4, 200_000, () => leaf.digest())
  measure('scalar wide record digest', 64, 20_000, () => record.digest())
  measure('scalar leaf stableHash', 4, 200_000, () => leaf.stableHash())

  const symbols = arrow.vectorFromArray(
    Array.from({ length: BATCH_ROWS }, (_, index) => (index % 2 === 0 ? 'AAPL' : 'MSFT')),
    new arrow.Utf8(),
  )
  const holder = new Field('row_digest', 'uint64', false, {
    'digest:role': 'holder',
    'digest:paths': '["symbol"]',
  })
  const root = new Field(
    'row',
    DataType.fromFields([new Field('symbol', 'utf8', false), holder]),
    false,
  )
  const missing = new arrow.Table({ symbol: symbols }).batches[0]
  const defaults = new arrow.Table({
    symbol: symbols,
    row_digest: arrow.vectorFromArray(Array(BATCH_ROWS).fill(0n), new arrow.Uint64()),
  }).batches[0]
  const populated = new arrow.Table({
    symbol: symbols,
    row_digest: arrow.vectorFromArray(
      Array.from({ length: BATCH_ROWS }, (_, index) => BigInt(index + 1)),
      new arrow.Uint64(),
    ),
  }).batches[0]
  const signedHolder = new Field('row_digest', 'int64', false, {
    'digest:role': 'holder',
    'digest:paths': '["symbol"]',
  })
  const signedRoot = new Field(
    'row',
    DataType.fromFields([new Field('symbol', 'utf8', false), signedHolder]),
    false,
  )
  const state = new xxhash.Xxh3(7n)
  measureRows('fillArrowBatch (missing holder)', BATCH_ROWS, 100, () => {
    state.fillArrowBatch(root, missing)
  })
  measureRows('fillArrowBatch (missing signed holder)', BATCH_ROWS, 100, () => {
    state.fillArrowBatch(signedRoot, missing)
  })
  measureRows('fillArrowBatch (default holders)', BATCH_ROWS, 100, () => {
    state.fillArrowBatch(root, defaults)
  })
  measureRows('fillArrowBatch (preserve populated)', BATCH_ROWS, 100, () => {
    state.fillArrowBatch(root, populated)
  })
  measureRows('fillArrowBatch (force populated)', BATCH_ROWS, 100, () => {
    state.fillArrowBatch(root, populated, true)
  })
}

main()
