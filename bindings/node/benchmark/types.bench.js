'use strict'

// Fast time + memory benchmark for the yggdryl.types schema layer (runs in ~1 s).
//
// Time: DataType construction + the category drill-down, Field construction with metadata, and
// Headers (the centralized metadata map) operations, all in Mops/s. Memory: the V8 heapUsed delta
// per live DataType / Field / Headers, validating the thin wrapper adds no runaway allocation.
//
// Build the addon in RELEASE first (npm run build). Run with --expose-gc for cleaner memory:
//
//   node --expose-gc bindings/node/benchmark/types.bench.js

const { types, io } = require('..')
const { DataType, Field } = types
const { Headers } = io

const ITERS = 50_000

const NAMES = ['u8', 'i32', 'i64', 'u96', 'i128', 'u256', 'f16', 'f64', 'utf8', 'binary']
const TYPES = NAMES.map((n) => DataType.byName(n))

function mopsS(items, iters, op) {
  op() // warm up
  const start = process.hrtime.bigint()
  for (let i = 0; i < iters; i++) op()
  const secs = Number(process.hrtime.bigint() - start) / 1e9
  return (items * iters) / secs / 1_000_000
}

function bytesPer(count, build) {
  if (global.gc) global.gc()
  const before = process.memoryUsage().heapUsed
  const kept = build()
  const per = (process.memoryUsage().heapUsed - before) / count
  void kept.length // keep alive
  return per
}

function main() {
  console.log(`yggdryl.types - time & memory (${ITERS} iters)\n`)

  const ops = [
    ['DataType.byName', () => { for (const n of NAMES) DataType.byName(n) }, NAMES.length],
    ['category drill-down', () => { for (const dt of TYPES) dt.isInteger() || dt.isFloating() || dt.isUtf8() }, TYPES.length],
    ['Field (no metadata)', () => { for (const dt of TYPES) new Field('col', dt, false) }, TYPES.length],
    ['Field (with metadata)', () => { for (const dt of TYPES) new Field('col', dt, false, { unit: 'count', source: 'x' }) }, TYPES.length],
    ['Headers build+read', () => { const h = new Headers(); h.insert('a', '1'); h.insert('b', '2'); h.get('a'); h.has('b'); h.toObject() }, 1],
  ]
  console.log('time (Mops/s):')
  for (const [name, op, items] of ops) {
    console.log(`  ${name.padEnd(26)} ${mopsS(items, ITERS, op).toFixed(2).padStart(7)}`)
  }

  const n = 20_000
  const perDt = bytesPer(n, () => Array.from({ length: n }, (_, i) => DataType.byName(NAMES[i % NAMES.length])))
  const perField = bytesPer(n, () => Array.from({ length: n }, (_, i) => new Field('c', TYPES[i % TYPES.length], false)))
  const perMeta = bytesPer(n, () => Array.from({ length: n }, (_, i) => new Headers({ k: String(i) })))
  console.log('\nmemory (V8 heapUsed delta):')
  console.log(`  ${'bytes / DataType'.padEnd(26)} ${perDt.toFixed(1).padStart(7)}`)
  console.log(`  ${'bytes / Field'.padEnd(26)} ${perField.toFixed(1).padStart(7)}`)
  console.log(`  ${'bytes / Headers'.padEnd(26)} ${perMeta.toFixed(1).padStart(7)}`)
}

main()
