'use strict'

// The replay's walk, its index and a scenario's re-run: `node/replay/walk.js`.

const assert = require('node:assert/strict')
const test = require('node:test')

const { graph } = require('yggdryl')

const { eventJson } = require('../../replay/json.js')
const { books, synthetic, T0 } = require('../../replay/synthetic.js')
const {
  bookAt,
  booksBetween,
  eventsOf,
  indexBooks,
  instantOf,
  merged,
  rerun,
  walk,
} = require('../../replay/walk.js')
const web = require('../../replay/web.js')

const MS = 1_000_000n
const hashes = (items) => items.map((item) => String(item.stableHash()))

function order(at, code, price = '82', facts = {}) {
  return new graph.OrderEvent(at, { crosscode: code, ticker: 'ALPHA', side: 'BUY', price, quantity: 5, ...facts })
}

test('instantOf: the grid step, else the event clock; an undated leaf has none', () => {
  const [first] = synthetic()
  assert.equal(instantOf(first), T0)
  assert.equal(instantOf(first.intoLeaf()), T0)
  const tick = books(2).find((book) => book.snapunix !== null)
  assert.equal(instantOf(tick), tick.snapunix)
  // A grid step read out of a later statement is placed at its step, not its clock.
  const stepped = new graph.OrderEvent(T0 + 5n, { crosscode: 'S', snapunix: T0 })
  assert.equal(instantOf(stepped), T0)
  assert.equal(instantOf(new graph.Order({ crosscode: 'O' })), null)
})

test('walk is the native BookIterator, and refuses what it refuses', () => {
  const operations = synthetic()
  assert.deepEqual(hashes(walk(operations)), hashes([...new graph.BookIterator(operations)]))
  assert.deepEqual(hashes(walk(operations, { global: true })), hashes([...new graph.BookIterator(operations, 0, true)]))
  assert.throws(
    () => walk([order(T0 + 5n, 'L'), order(T0, 'E')]),
    /\$\.operations: expected a sorted operation timestamp at or after 1700000000000000005, got 1700000000000000000/,
  )
})

test('indexBooks and booksBetween: per symbol, by instant, bounds inclusive', () => {
  const walked = books()
  const index = indexBooks(walked)
  assert.deepEqual([...index.bySymbol.keys()], ['ALPHA', 'BETA'])
  assert.equal(index.all.books.length, walked.length)
  const alpha = index.bySymbol.get('ALPHA')
  assert.ok(alpha.books.every((book) => book.crosscode === 'ALPHA'))
  assert.deepEqual(alpha.instants, alpha.books.map((book) => book.currunix))

  // One nanosecond apart, two books; each bound is inclusive.
  const between = booksBetween(index, 'ALPHA', T0 + MS, T0 + MS + 1n)
  assert.deepEqual(between.map((book) => book.currunix), [T0 + MS, T0 + MS + 1n])
  assert.deepEqual(booksBetween(index, 'ALPHA', T0 + MS + 1n, T0 + MS + 1n).map((book) => book.currunix), [T0 + MS + 1n])
  assert.deepEqual(booksBetween(index, 'ALPHA', T0 + MS + 2n, T0 + 2n * MS), [])
  assert.deepEqual(booksBetween(index, 'ALPHA', T0 + 2n * MS, T0 + MS), [])
  // Every symbol when none is named, in walk order; an unknown symbol answers none.
  assert.deepEqual(hashes(booksBetween(index)), hashes(walked))
  assert.deepEqual(
    booksBetween(index, undefined, T0, T0).map((book) => book.crosscode),
    ['ALPHA', 'BETA'],
  )
  assert.deepEqual(booksBetween(index, 'GAMMA'), [])
})

test('bookAt: the book standing at an instant', () => {
  const index = indexBooks(books())
  assert.equal(bookAt(index, 'ALPHA', T0 - 1n), null)
  assert.equal(bookAt(index, 'ALPHA', T0).currunix, T0)
  assert.equal(bookAt(index, 'ALPHA', T0 + MS + 1n).currunix, T0 + MS + 1n)
  // Between two books, the earlier one stands.
  assert.equal(bookAt(index, 'ALPHA', T0 + 2n * MS).currunix, T0 + MS + 1n)
  assert.equal(bookAt(index, 'BETA').currunix, T0 + 6n * MS)
  assert.equal(bookAt(index, 'GAMMA', T0), null)
})

test('merged keeps base before inserted on a tie, and each stream in its order', () => {
  const base = [order(T0, 'B1'), order(T0 + MS, 'B2'), order(T0 + MS, 'B3'), order(T0 + 2n * MS, 'B4')]
  const inserted = [order(T0 - 1n, 'I0'), order(T0 + MS, 'I1'), order(T0 + MS, 'I2'), order(T0 + 3n * MS, 'I3')]
  assert.deepEqual(
    merged(base, inserted).map((item) => item.crosscode),
    ['I0', 'B1', 'B2', 'B3', 'I1', 'I2', 'B4', 'I3'],
  )
  assert.deepEqual(merged(base, []), base)
  assert.deepEqual(merged([], inserted), inserted)
  // An undated base item keeps its place ahead of what follows it.
  const undated = new graph.Order({ crosscode: 'U' })
  assert.deepEqual(
    merged([order(T0, 'B1'), undated, order(T0 + 2n * MS, 'B2')], [order(T0 + MS, 'I1')]).map((item) => item.crosscode),
    ['B1', 'U', 'I1', 'B2'],
  )
})

test('insert then remove restores every base book exactly', async () => {
  const { createScenario, insertEvent, removeEvent } = await web()
  const base = synthetic()
  const baseHashes = hashes(walk(base))

  // An order inserted at 3.5 ms changes the ALPHA books from there on.
  const inserted = order(T0 + 3n * MS + 500_000n, 'ALPHA-X-1', '82.25')
  let scenario = insertEvent(createScenario('what-if'), eventJson(inserted))
  assert.deepEqual(hashes(eventsOf(scenario)), hashes([inserted]))
  const changed = await rerun(base, scenario)
  assert.equal(changed.from, T0 + 3n * MS + 500_000n)
  assert.equal(changed.books.length, baseHashes.length + 1)
  const later = changed.books.filter((book) => book.crosscode === 'ALPHA' && book.currunix >= changed.from)
  assert.ok(later.every((book) => book.bid.limits.some((limit) => limit.price === '82.25')))
  // Before the earliest affected instant, every book is the base's.
  const before = changed.books.filter((book) => book.currunix < changed.from)
  assert.deepEqual(hashes(before), baseHashes.slice(0, before.length))

  scenario = removeEvent(scenario, inserted.curruuid)
  const restored = await rerun(base, scenario)
  assert.equal(restored.from, null)
  assert.deepEqual(hashes(restored.books), baseHashes)
})

test('a scenario orders its events at the instant the walk reads them: snapunix, else currunix', async () => {
  const { createScenario, insertEvent } = await web()
  const base = synthetic()
  // Stated at 3 ms but read at its 4 ms grid step: after an event stated at 3.5 ms.
  const stepped = order(T0 + 3n * MS, 'ALPHA-X-2', '82.1', { snapunix: T0 + 4n * MS })
  const plain = order(T0 + 3n * MS + 500_000n, 'ALPHA-X-1', '82.25')
  for (const inserted of [[stepped, plain], [plain, stepped]]) {
    let scenario = createScenario('stepped')
    for (const leaf of inserted) scenario = insertEvent(scenario, eventJson(leaf))
    assert.deepEqual(scenario.events.map((event) => event.crosscode), ['ALPHA-X-1', 'ALPHA-X-2'])
    // The merged stream is the walk's order, so the re-run walks it.
    const { books: walked, from } = await rerun(base, scenario)
    assert.equal(walked.length, walk(base).length + 2)
    // The earliest affected instant is the earliest instant the walk reads an inserted event at:
    // the stepped event folds at its 4 ms grid step, so the plain event at 3.5 ms comes first.
    assert.equal(from, T0 + 3n * MS + 500_000n)
    const alpha = walked.filter((book) => book.crosscode === 'ALPHA' && instantOf(book) > T0 + 3n * MS)
    assert.deepEqual(alpha.map(instantOf).slice(0, 2), [T0 + 3n * MS + 500_000n, T0 + 4n * MS])
  }
})

test('a re-run walks the operations it is handed, decoding no event again', async () => {
  const { createScenario, insertEvent } = await web()
  const base = synthetic()
  const inserted = order(T0 + 3n * MS + 500_000n, 'ALPHA-X-1', '82.25')
  // The event's text is no leaf: only the leaf handed beside it is walked.
  const scenario = insertEvent(createScenario('handed'), { ...eventJson(inserted), native: 'bm90IGFycm93' })
  const handed = await rerun(base, scenario, {}, [inserted])
  const decoded = await rerun(base, insertEvent(createScenario('handed'), eventJson(inserted)))
  assert.deepEqual(hashes(handed.books), hashes(decoded.books))
  assert.equal(handed.from, decoded.from)
  await assert.rejects(rerun(base, scenario))
  await assert.rejects(rerun(base, scenario, {}, []), {
    name: 'TypeError',
    message: 'expected one operation per scenario event: 1 events, 0 operations',
  })
})

test('a re-run refuses what the walk refuses, verbatim', async () => {
  const { createScenario, insertEvent } = await web()
  const base = synthetic()
  // An inserted book side is no operation.
  const side = new graph.BookSide('BUY')
  const scenario = insertEvent(createScenario('bad'), { ...eventJson(side), currunix: String(T0) })
  await assert.rejects(rerun(base, scenario), /\$\.operation\.kind: expected order_event/)
})
