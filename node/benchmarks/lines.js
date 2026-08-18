'use strict'

// The Arrow line projection across a rotated log folder, measured at the
// JavaScript boundary.
//
// WHAT A NUMBER HERE INCLUDES, AND WHY IT IS NOT THE PARSER'S THROUGHPUT.
// Apache Arrow JS has no Arrow C Data consumer, so this binding hands every
// batch across as its own self-contained Arrow IPC stream: the native side
// encodes each batch and Arrow JS decodes it again before a single column can
// be read. Every row printed below therefore carries a per-batch encode and
// decode that the Rust Criterion `io`/`lines_arrow` groups and the Python
// `log_lines.py` rows - which cross the Arrow C Stream interface, lazily and
// without a copy - never pay. Read these rows against each other, never
// against a Rust or Python number for the same corpus.
//
// That copied boundary is also why the default corpus is 250,000 records
// rather than the 1,000,000 the Rust and Python targets default to: the same
// wall-clock budget buys fewer records here. The record count is printed in
// the header so nobody reads a Node row beside a 1M-record Rust row.
//
// Throughput is reported in DECODED bytes - the text the parser actually
// consumes - and in rows/s. The gzip wire size is printed once, labelled, and
// is never the denominator of a throughput: compressing the fixture harder
// would otherwise "speed up" the parser.
//
// HOW TO READ A DIFFERENCE BETWEEN TWO ROWS. Every pass over the whole corpus
// is timed on its own and a row prints the MEDIAN and the BEST of
// `YGGDRYL_BENCH_ITERATIONS` passes (5 by default), the way
// `python/benchmarks/log_lines.py` reports, plus the SPREAD - the slowest pass
// minus the fastest, over the median. Read the spread before reading any
// difference: on a shared machine one row here moves by a few percent between
// runs, which is the same size as the coding, folder-shape, and cast deltas
// below, and a delta smaller than the spread printed beside it is noise with a
// sign, not a result. The Rust `io` target's `lines_gzip` group settles those
// three over the same corpus with Criterion's sampled confidence intervals;
// that is the number to quote when the two rows here overlap.
//
// One row is evidence for exactly one claim:
//
//   readArrowLines folder gzip   the production shape: 8 rotated `.log.gz`
//                                leaves under one directory handle, read in
//                                name-sorted order, each leaf decoded as a
//                                stream and restarting `rownum` at 1.
//   readArrowLines folder plain  the same records as uncompressed leaves, drained
//                                the same way. The difference against the row
//                                above is the NET cost of storing the corpus
//                                gzip-coded on local storage, not the inflate
//                                in isolation: the plain leaves move the whole
//                                decoded payload off disk while the coded ones
//                                move about a fifth of it and then inflate, and
//                                those two effects have opposite signs. It is
//                                the trade a production reader actually makes;
//                                for inflate alone, both sides have to move
//                                identical source bytes, which is what the Rust
//                                `lines_arrow/parse/{plain,gzip}` pair does with
//                                in-memory handles.
//   readArrowLines single gzip   the same records in ONE `.log.gz`. The
//                                difference against the folder gzip row is
//                                what the folder shape costs - per-leaf open,
//                                media-type inference, and the batch boundary
//                                forced at every leaf - and nothing else.
//   readArrowLines folder utf8   the same drain of the same gzip folder with
//                                `captureTypes` declaring `thread_id` and
//                                `latency_us` as `utf8`, which turns the strict
//                                native cast OFF. Nothing but the cast differs
//                                from the first row, so that difference is what
//                                typing two captures on every row costs
//                                (2 x RECORDS values) - the same pairing the
//                                Rust `lines_gzip/casts/typed` and
//                                `lines_gzip/casts/text` cases make, both of
//                                which only count rows too.
//   typed accessors              the FIRST row's parse plus an aggregate read
//                                through the typed column. `latency_us` infers
//                                `int64` from its own `\d+` sub-pattern, so
//                                Arrow JS hands out BigInt and the sum is
//                                BigInt arithmetic with no per-row string
//                                parsing. Minus the first row, that is the
//                                JavaScript aggregation over a typed column.
//   text captures + js cast      the FOURTH row's parse plus the same aggregate
//                                over the utf8 column, one `BigInt(string)` per
//                                matched row. Minus the fourth row, that is the
//                                same aggregation with the conversion moved
//                                into JavaScript.
//
// The two aggregate rows are not each other's baseline and their difference is
// not the price of the typed accessor: they parse differently (cast on against
// cast off) and they convert different volumes - the native side types BOTH
// captures on EVERY row, while the JavaScript side converts `latency_us` alone
// and only on the one row in three where `level` is `ee`. Each aggregate row is
// read against the parse-only row it shares its options with; the cast itself
// is row 1 against row 4.
//
// Nothing quadratic is claimed from one corpus size: `--records` sweeps it
// (throughput per decoded byte should stay flat), and the Rust `io` target
// carries the scale sweep as a Criterion group.
//
//     node benchmarks/lines.js [--records 250000]
//     YGGDRYL_BENCH_ITERATIONS=15 node benchmarks/lines.js
//
// A binding benchmark measures a RELEASE addon; a debug build understates the
// native side by an order of magnitude (`npm run build`, not `build:debug`).

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const { performance } = require('node:perf_hooks')
const path = require('node:path')
const zlib = require('node:zlib')
const { IOBase } = require('yggdryl')

function integerFlag(flag, fallback) {
  const at = process.argv.indexOf(flag)
  if (at < 0) return fallback
  const value = Number.parseInt(process.argv[at + 1] ?? '', 10)
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new RangeError(`${flag} must be a positive safe integer`)
  }
  return value
}

// One whole corpus per pass is a second of work, so this target counts passes
// rather than the thousands `io.js` and `records.js` run. Five is the floor at
// which a median and a spread say anything; raise it when a difference between
// two rows sits inside their spread and has to be resolved here.
const iterations = Number.parseInt(process.env.YGGDRYL_BENCH_ITERATIONS ?? '5', 10)
if (!Number.isSafeInteger(iterations) || iterations <= 0) {
  throw new RangeError('YGGDRYL_BENCH_ITERATIONS must be a positive safe integer')
}

const LEAVES = 8
const RECORDS = integerFlag('--records', 250_000)
if (RECORDS % LEAVES !== 0) {
  throw new RangeError(`--records must divide evenly across ${LEAVES} rotated leaves`)
}

// The shared corpus pattern, spelled identically in Rust, Python, and here.
// `level` and `logger` are utf8; `thread_id` and `latency_us` type themselves
// as int64 off their own `\d+` sub-patterns - that inference is the claim the
// last two rows price.
const PATTERN =
  '^\\d{4}-\\d{2}-\\d{2} \\d{2}:\\d{2}:\\d{2}\\S*' +
  ' \\[(?<level>[^\\]]+)\\] \\[(?<logger>[^\\]]+)\\]' +
  ' \\[(?<thread_id>\\d+)\\] took=(?<latency_us>\\d+)'

const LEVELS = ['ii', 'ww', 'ee']
const LOGGERS = ['engine', 'router', 'ledger', 'feed']

/** The shared corpus generator: byte-for-byte the Rust and Python one. */
function record(index) {
  const minute = Math.floor(index / 3_600) % 60
  const second = Math.floor(index / 60) % 60
  const micro = index % 1_000_000
  const level = LEVELS[index % 3]
  const logger = LOGGERS[index % 4]
  const thread = index % 16
  const latency = 40 + (index % 960)
  const qty = 100 + (index % 900)
  const symbol = index % 128
  const price = 187 + (index % 400) / 100
  let line =
    `2024-02-01 10:${String(minute).padStart(2, '0')}:${String(second).padStart(2, '0')}` +
    `.${String(micro).padStart(6, '0')} [${level}] [${logger}] [${thread}]` +
    ` took=${latency} fill ${qty} SYMB-${String(symbol).padStart(4, '0')}` +
    ` @ ${price.toFixed(2)} order=${String(index).padStart(8, '0')}\n`
  // Every 50th record is a stack trace whose two continuation lines fold into
  // the SAME row (`lines` reads 3). A naive splitlines loop would count them
  // as rows of their own, which is why the row count is asserted below.
  if (index % 50 === 49) {
    line += '    at engine::match(order.rs:118)\n    at engine::step(order.rs:64)\n'
  }
  return line
}

/** The text of one rotated leaf, as a global record index range. */
function leafText(from, to) {
  const lines = new Array(to - from)
  for (let index = from; index < to; index += 1) lines[index - from] = record(index)
  return lines.join('')
}

// Every fixture is written once, outside every measured loop: what the rows
// below time is the read, never the generator or the gzip encoder.
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-node-lines-'))
try {
  const gzipFolder = path.join(root, 'rotated-gzip')
  const plainFolder = path.join(root, 'rotated-plain')
  const single = path.join(root, 'single', 'app.log.gz')
  fs.mkdirSync(gzipFolder, { recursive: true })
  fs.mkdirSync(plainFolder, { recursive: true })
  fs.mkdirSync(path.dirname(single), { recursive: true })

  const perLeaf = RECORDS / LEAVES
  let decodedBytes = 0
  let wireBytes = 0
  const whole = new Array(LEAVES)
  for (let leaf = 0; leaf < LEAVES; leaf += 1) {
    const text = leafText(leaf * perLeaf, (leaf + 1) * perLeaf)
    whole[leaf] = text
    const coded = zlib.gzipSync(text)
    fs.writeFileSync(path.join(gzipFolder, `app-${leaf}.log.gz`), coded)
    fs.writeFileSync(path.join(plainFolder, `app-${leaf}.log`), text)
    decodedBytes += Buffer.byteLength(text)
    wireBytes += coded.length
  }
  fs.writeFileSync(single, zlib.gzipSync(whole.join('')))
  const singleWireBytes = fs.statSync(single).size

  const gzipHandle = new IOBase(gzipFolder)
  const plainHandle = new IOBase(plainFolder)
  const singleHandle = new IOBase(single)
  // Declaring the two inferred captures as utf8 turns the strict native cast
  // off, which is exactly what the JavaScript-side conversion then replaces.
  const asText = { captureTypes: { thread_id: 'utf8', latency_us: 'utf8' } }

  /** Drain the projection, counting rows the way a consumer would. */
  function count(handle, options) {
    let rows = 0
    for (const batch of handle.readArrowLines(PATTERN, options ?? null)) {
      rows += batch.numRows
    }
    return rows
  }

  /**
   * Aggregate through the typed int64 capture: BigInt arithmetic, no parsing.
   */
  function aggregateTyped(handle) {
    let rows = 0
    let matched = 0
    let total = 0n
    for (const batch of handle.readArrowLines(PATTERN)) {
      const level = batch.getChild('level')
      const latency = batch.getChild('latency_us')
      rows += batch.numRows
      for (let index = 0; index < batch.numRows; index += 1) {
        if (level.get(index) !== 'ee') continue
        matched += 1
        total += latency.get(index)
      }
    }
    return { matched, rows, total }
  }

  /**
   * The same aggregate over utf8 captures: every value is converted here.
   */
  function aggregateText(handle) {
    let rows = 0
    let matched = 0
    let total = 0n
    for (const batch of handle.readArrowLines(PATTERN, asText)) {
      const level = batch.getChild('level')
      const latency = batch.getChild('latency_us')
      rows += batch.numRows
      for (let index = 0; index < batch.numRows; index += 1) {
        if (level.get(index) !== 'ee') continue
        matched += 1
        total += BigInt(latency.get(index))
      }
    }
    return { matched, rows, total }
  }

  // A parser that silently dropped or split rows would still look fast, so
  // every corpus is checked - loudly, and outside the timed loops - to yield
  // exactly RECORDS rows rather than RECORDS plus the continuation lines.
  assert.equal(count(gzipHandle), RECORDS, 'gzip folder must yield RECORDS rows')
  assert.equal(count(plainHandle), RECORDS, 'plain folder must yield RECORDS rows')
  assert.equal(count(singleHandle), RECORDS, 'single gzip file must yield RECORDS rows')
  const typed = aggregateTyped(gzipHandle)
  const text = aggregateText(gzipHandle)
  assert.equal(typed.rows, RECORDS)
  // `text` drains the utf8 projection the `readArrowLines folder utf8` row
  // times, so that row's row count is proven here too.
  assert.equal(text.rows, RECORDS)
  // The two accessors must agree on the answer, or the cheaper one is only
  // cheaper because it computed something else.
  assert.equal(typed.matched, text.matched)
  assert.equal(typed.total, text.total)
  // `ee` is the third entry of the level cycle, so exactly one record in
  // every whole cycle of three carries it.
  assert.equal(typed.matched, Math.floor(RECORDS / 3))

  /**
   * Time every pass on its own and report the median, the best, and the spread
   * across them. A mean of a handful of whole-corpus passes hides exactly the
   * dispersion a reader needs to decide whether the small differences between
   * these rows carry a sign; rates are quoted off the median.
   */
  function benchmark(name, rows, bytes, operation) {
    operation()
    const samples = new Array(iterations)
    for (let index = 0; index < iterations; index += 1) {
      const started = performance.now()
      operation()
      samples[index] = performance.now() - started
    }
    samples.sort((left, right) => left - right)
    const middle = samples.length >> 1
    const median =
      samples.length % 2 === 1
        ? samples[middle]
        : (samples[middle - 1] + samples[middle]) / 2
    const best = samples[0]
    const spread = ((samples[samples.length - 1] - best) / median) * 100
    const seconds = median / 1_000
    const rowRate = Math.round(rows / seconds).toLocaleString('en-US')
    const throughput = bytes / seconds / (1024 * 1024)
    console.log(
      `${`${name}:`.padEnd(30)} ${median.toFixed(3).padStart(10)} ms median` +
        ` ${best.toFixed(3).padStart(10)} ms best ${rowRate.padStart(12)} rows/s` +
        ` ${throughput.toFixed(1).padStart(7)} MiB/s decoded` +
        ` ${spread.toFixed(1).padStart(5)}% spread`,
    )
  }

  console.log(
    `Node ${process.version}; ${RECORDS.toLocaleString('en-US')} records` +
      ` (NOT the 1,000,000 the Rust and Python targets default to),` +
      ` ${LEAVES} rotated leaves`,
  )
  console.log(
    `median and best of ${iterations} timed whole-corpus passes per row` +
      ` (one untimed warm-up first); spread is (slowest - fastest) / median, and` +
      ` a difference between two rows smaller than their spread is not a result`,
  )
  console.log(
    `${decodedBytes.toLocaleString('en-US')} decoded bytes` +
      ` (gzip wire: ${wireBytes.toLocaleString('en-US')} across the folder,` +
      ` ${singleWireBytes.toLocaleString('en-US')} as one file - wire bytes are never` +
      ` the throughput denominator)`,
  )
  console.log(
    'Every batch crosses as its own copied Arrow IPC stream - Arrow JS has no',
  )
  console.log(
    'Arrow C Data consumer - so each row below includes a per-batch encode and',
  )
  console.log(
    'decode that the Rust and Python numbers do not pay. These are boundary',
  )
  console.log('numbers, not the parser\'s throughput.')
  console.log('-'.repeat(80))

  // Keep these row spellings stable so before/after runs stay comparable; add
  // a case beside them rather than renaming one.
  benchmark('readArrowLines folder gzip', RECORDS, decodedBytes, () => count(gzipHandle))
  benchmark('readArrowLines folder plain', RECORDS, decodedBytes, () => count(plainHandle))
  benchmark('readArrowLines single gzip', RECORDS, decodedBytes, () => count(singleHandle))
  // The same drain as the first row with the strict cast declared away: the
  // pair isolates the native cast, and it is what the `text captures + js cast`
  // aggregate below subtracts against - that row parses this way, not the way
  // the first row does.
  benchmark('readArrowLines folder utf8', RECORDS, decodedBytes, () => count(gzipHandle, asText))
  benchmark('typed accessors', RECORDS, decodedBytes, () => aggregateTyped(gzipHandle))
  benchmark('text captures + js cast', RECORDS, decodedBytes, () => aggregateText(gzipHandle))
} finally {
  fs.rmSync(root, { force: true, recursive: true })
}
