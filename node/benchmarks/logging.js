'use strict'

// A smoke-sized view of the logging boundary: what a JavaScript call costs
// before the core sees a record, and what it costs once it does. Increase the
// iteration count explicitly when collecting publication numbers.

const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { performance } = require('node:perf_hooks')
const { logging } = require('yggdryl')

const iterations = Number.parseInt(process.env.YGGDRYL_BENCH_ITERATIONS ?? '20000', 10)
if (!Number.isSafeInteger(iterations) || iterations <= 0) {
  throw new RangeError('YGGDRYL_BENCH_ITERATIONS must be a positive safe integer')
}

const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-bench-logging-'))
const quiet = logging.getLogger('bench.quiet')
quiet.propagate = false
quiet.setLevel('WARNING')
const dropped = logging.getLogger('bench.null')
dropped.propagate = false
dropped.setLevel('INFO')
dropped.addHandler(new logging.NullHandler())
const filed = logging.getLogger('bench.file')
filed.propagate = false
filed.setLevel('INFO')
const handler = new logging.FileHandler(path.join(folder, 'bench.log'), { capacity: 1 << 16 })
filed.addHandler(handler)
const repeated = logging.getLogger('bench.repeated')
repeated.propagate = false
repeated.setLevel('INFO')
repeated.deduplicating = true
repeated.addHandler(new logging.NullHandler())

const cases = [
  // The level read through the cached threshold, then nothing.
  ['disabled debug', () => quiet.debug('below the level')],
  // The call site walked and one record handed to a handler that drops it.
  ['enabled info to a NullHandler', () => dropped.info('opened 3 venues')],
  // The same record spelled in the terminal format and held under 64 KiB.
  ['enabled info to a FileHandler (64 KiB)', () => filed.info('opened 3 venues')],
  // A repeat counted by its hash and dropped before any handler.
  ['deduplicated repeat', () => repeated.info('opened 3 venues')],
]

for (const [name, operation] of cases) {
  operation()
  const started = performance.now()
  for (let index = 0; index < iterations; index += 1) operation()
  const nanoseconds = ((performance.now() - started) * 1_000_000) / iterations
  console.log(`${name}: ${nanoseconds.toFixed(1)} ns/op`)
}
handler.close()
fs.rmSync(folder, { recursive: true, force: true })
