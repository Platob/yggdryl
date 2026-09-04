'use strict'

// Plain-text records through the generic JavaScript record/media boundary.
//
//     npm run --prefix node bench:text -- --records 100000
//
// Build the release addon first. Every BatchReader batch crosses as copied
// Arrow IPC because Arrow JS has no C Data consumer.

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { performance } = require('node:perf_hooks')
const zlib = require('node:zlib')

const { IOBase, TextOptions } = require('yggdryl')

function positiveArgument(name, fallback) {
  const index = process.argv.indexOf(name)
  if (index < 0) return fallback
  const value = Number.parseInt(process.argv[index + 1] ?? '', 10)
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new RangeError(`${name} must be a positive safe integer`)
  }
  return value
}

const rows = positiveArgument('--records', 100_000)
const iterations = positiveArgument(
  '--iterations',
  Number.parseInt(process.env.YGGDRYL_BENCH_ITERATIONS ?? '5', 10),
)
const rowheader = '^(?<stamp>\\S+) \\[(?<level>[A-Z]+)\\] id=(?<id>\\d+)'

function corpus(from, to) {
  const lines = new Array(to - from)
  for (let index = from; index < to; index += 1) {
    const level = index % 3 === 0 ? 'WARN' : 'INFO'
    lines[index - from] =
      `2026-08-14T00:05:${String(index % 60).padStart(2, '0')} ` +
      `[${level}] id=${index} event ${index}\n`
  }
  return lines.join('')
}

function textOptions() {
  const options = new TextOptions()
  options.rowheader = rowheader
  options.lstrip = '^\\s+'
  options.rstrip = '\\s+$'
  return options
}

const options = textOptions()
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-text-bench-'))

function countBatches(handle) {
  handle.intoText(options)
  let count = 0
  for (const batch of handle.readArrowReader()) count += batch.numRows
  return count
}

function countRecords(handle) {
  handle.intoText(options)
  let count = 0
  for (const row of handle.readRecords()) {
    assert.ok(row.body instanceof Uint8Array)
    count += 1
  }
  return count
}

function benchmark(name, operation) {
  operation()
  const samples = []
  for (let index = 0; index < iterations; index += 1) {
    const started = performance.now()
    const count = operation()
    assert.equal(count, rows)
    samples.push(performance.now() - started)
  }
  samples.sort((left, right) => left - right)
  const median = samples[Math.floor(samples.length / 2)]
  console.log(
    `${name.padEnd(34)} ${median.toFixed(3).padStart(10)} ms ` +
      `${Math.round(rows / (median / 1_000)).toLocaleString('en-US').padStart(12)} rows/s`,
  )
}

try {
  const plain = path.join(root, 'events.log')
  const coded = path.join(root, 'events.log.gz')
  const folder = path.join(root, 'rotated')
  fs.mkdirSync(folder)
  const text = corpus(0, rows)
  fs.writeFileSync(plain, text)
  fs.writeFileSync(coded, zlib.gzipSync(text))
  const middle = Math.floor(rows / 2)
  fs.writeFileSync(path.join(folder, 'a.log'), corpus(0, middle))
  fs.writeFileSync(path.join(folder, 'b.log.gz'), zlib.gzipSync(corpus(middle, rows)))

  const plainHandle = new IOBase(plain)
  const codedHandle = new IOBase(coded)
  const folderHandle = new IOBase(folder)
  assert.equal(countBatches(plainHandle), rows)
  assert.equal(countBatches(codedHandle), rows)
  assert.equal(countBatches(folderHandle), rows)
  assert.equal(countRecords(plainHandle), rows)

  console.log(
    `Node ${process.version}; ${rows.toLocaleString('en-US')} physical text rows; ` +
      `median of ${iterations} passes`,
  )
  benchmark('text/read_arrow_reader plain', () => countBatches(plainHandle))
  benchmark('text/read_arrow_reader gzip', () => countBatches(codedHandle))
  benchmark('text/read_arrow_reader folder', () => countBatches(folderHandle))
  benchmark('text/read_records plain', () => countRecords(plainHandle))
} finally {
  fs.rmSync(root, { force: true, recursive: true })
}
