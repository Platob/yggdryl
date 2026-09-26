'use strict'

// Where a replay's operations come from: `node/replay/sources.js`.

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { after, test } = require('node:test')
const { pathToFileURL } = require('node:url')

const { IOBase, TextOptions, fix, graph } = require('yggdryl')

const { captureNamesOf, loadSource, sendingTimeOf, SEED_SENDING_TIME } = require('../../replay/sources.js')
const { books, synthetic, T0 } = require('../../replay/synthetic.js')
const { walk } = require('../../replay/walk.js')

const ROOT = path.join(__dirname, '..', '..', '..')
// A second of a ULBridge's own capture, anonymized: the corpus
// `rust/tests/fix/ulbridge.rs` reads, under the committed dictionary.
const CAPTURE = path.join(ROOT, 'rust', 'tests', 'fix', 'ulbridge.log')
const SEED = path.join(ROOT, 'config', 'fix')

let registry
function seed() {
  registry ??= fix.FixRegistry.fromHandle(SEED)
  return registry.clone()
}

const made = []
after(() => {
  for (const folder of made) fs.rmSync(folder, { recursive: true, force: true })
})

function scratch() {
  const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-replay-'))
  made.push(folder)
  return folder
}

const hashes = (items) => items.map((item) => String(item.stableHash()))

test('the ULBridge capture: eleven operations and seven books, the last the Rust pin', () => {
  const source = loadSource(CAPTURE, { registry: seed() })
  assert.equal(source.kind, 'fix')
  assert.equal(source.id, 'ulbridge.log')
  assert.equal(source.name, 'ulbridge.log')
  // Eight fills and three orders reach a book; the one admitted message the
  // core refused is the trade capture whose side states no `Side(54)`.
  assert.equal(source.operations.length, 11)
  const census = {}
  for (const operation of source.operations) census[operation.kind] = (census[operation.kind] ?? 0) + 1
  assert.deepEqual(census, { execution_event: 8, order_event: 3 })
  assert.equal(source.refusals.length, 1)
  assert.match(
    source.refusals[0],
    /invalid record value at \$\.NoSides\(552\)\[0\]\.Side\(54\): expected a bid or ask side, got no value/,
  )
  // The capture is read from its bytes under the seed clock, so an undated
  // line is dated by the codec and never by the file's mtime: the last book
  // is the one `rust/tests/fix/ulbridge.rs` pins.
  const walked = walk(source.operations)
  assert.equal(walked.length, 7)
  const last = walked.at(-1)
  assert.equal(last.ticker, '2454')
  assert.equal(last.stableHash(), 4_619_727_780_541_450_139n)
  assert.equal(last.currhashcode, 4_619_727_780_541_450_139n)
  assert.deepEqual(source.symbols, [...new Set(walked.map((book) => book.crosscode))].sort())

  // The clock may be named by its text.
  const again = loadSource(CAPTURE, { registry: seed(), sendingTime: String(SEED_SENDING_TIME) })
  assert.deepEqual(hashes(again.operations), hashes(source.operations))
  // The capture is any location the native handle opens: its file URL reads the same bytes.
  const located = loadSource(pathToFileURL(CAPTURE).href, { registry: seed() })
  assert.deepEqual([located.kind, located.name], ['fix', 'ulbridge.log'])
  assert.deepEqual(hashes(located.operations), hashes(source.operations))
})

test('a missing capture reads as the empty stream, as every missing store does', () => {
  const source = loadSource(path.join(scratch(), 'missing.log'), { registry: seed() })
  assert.deepEqual([source.kind, source.operations, source.symbols, source.refusals], ['fix', [], [], []])
})

test('the row header names the codec\'s captures, as the native text options state them', () => {
  const options = new TextOptions()
  options.rowheader = fix.ULBRIDGE_ROWHEADER
  assert.deepEqual(captureNamesOf(options), [
    'timestamp',
    'msgthreadid',
    'msgsessionid',
    'msgctxid',
    'msgseqnum',
    'msgpluginid',
    'level',
  ])
})

test('the sending clock: nanoseconds, their text, ISO text, or absence', () => {
  assert.equal(sendingTimeOf(null), null)
  const seedClock = sendingTimeOf(SEED_SENDING_TIME)
  assert.ok(seedClock.equals(sendingTimeOf(String(SEED_SENDING_TIME))))
  assert.ok(seedClock.equals(sendingTimeOf('2024-01-02T10:15:30Z')))
  assert.throws(() => sendingTimeOf('yesterday'), /expected an ISO date/)
})

test('the synthetic source is the synthetic scenario', () => {
  const source = loadSource('synthetic')
  assert.deepEqual(
    { id: source.id, kind: source.kind, name: source.name, symbols: source.symbols, refusals: source.refusals },
    { id: 'synthetic', kind: 'synthetic', name: 'synthetic', symbols: ['ALPHA', 'BETA'], refusals: [] },
  )
  assert.deepEqual(hashes(source.operations), hashes(synthetic()))
})

test('a Parquet round trip of the synthetic stream loads back equal by hash', () => {
  const file = path.join(scratch(), 'synthetic.parquet')
  IOBase.from(file).overwriteArrowReader(graph.MarketData.arrowReader(synthetic()))
  const source = loadSource(file)
  assert.equal(source.kind, 'parquet')
  assert.equal(source.id, 'synthetic.parquet')
  assert.deepEqual(hashes(source.operations), hashes(synthetic()))
  assert.deepEqual(hashes(walk(source.operations)), hashes(books()))
  assert.deepEqual(source.symbols, ['ALPHA', 'BETA'])
})

test('an Arrow file whose instants go back is refused, naming both', () => {
  const file = path.join(scratch(), 'backwards.arrow')
  const later = new graph.OrderEvent(T0 + 5n, { crosscode: 'L', side: 'BUY', price: '1', quantity: 1, ticker: 'X' })
  const earlier = new graph.OrderEvent(T0, { crosscode: 'E', side: 'BUY', price: '1', quantity: 1, ticker: 'X' })
  IOBase.from(file).overwriteArrowReader(graph.MarketData.arrowReader([later, earlier]))
  assert.throws(() => loadSource(file), {
    name: 'RangeError',
    message:
      'source backwards.arrow: operations[1] at 1700000000000000000 comes before operations[0] at 1700000000000000005: ' +
      'expected nondecreasing instants, snapunix else currunix',
  })
  // In order, the same two load.
  const sorted = path.join(scratch(), 'sorted.feather')
  IOBase.from(sorted).overwriteArrowReader(graph.MarketData.arrowReader([earlier, later]))
  assert.equal(loadSource(sorted).operations.length, 2)
})

test('a missing file reads as the empty stream, as every missing store does', () => {
  const source = loadSource(path.join(scratch(), 'missing.parquet'))
  assert.deepEqual([source.kind, source.operations, source.symbols], ['parquet', [], []])
})

test('what the walk cannot fold is refused by the walk, verbatim', () => {
  const file = path.join(scratch(), 'undated.ipc')
  IOBase.from(file).overwriteArrowReader(graph.MarketData.arrowReader([new graph.Order({ crosscode: 'O' })]))
  assert.throws(() => loadSource(file), /\$\.operation\.kind: expected order_event, quote_event, execution_event, trade_event or snapshot_event, got order/)
  assert.throws(() => loadSource(42), { name: 'TypeError', message: "expected 'synthetic' or a file path, got number" })
})
