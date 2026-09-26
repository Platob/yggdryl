'use strict'

// The replay's walk and its index: `node/replay/walk.js`.

const assert = require('node:assert/strict')
const test = require('node:test')

const { graph } = require('yggdryl')

const { books, synthetic, T0 } = require('../../replay/synthetic.js')
const { bookAt, booksBetween, indexBooks, instantOf, walk } = require('../../replay/walk.js')

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

