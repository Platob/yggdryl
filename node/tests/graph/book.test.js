'use strict'

// Books, their readings by side, snapshot controls and the book walk:
// `node/src/graph/book.rs`.

const assert = require('node:assert/strict')
const test = require('node:test')

const { Side, graph } = require('yggdryl')

const CLOCK = 1_700_000_000_000_000_000n

function order(clock = CLOCK, price = '101', code = 'O-1') {
  return new graph.OrderEvent(clock, { crosscode: code, side: 'BUY', price, quantity: 10, ticker: 'IBM' })
}

function quote(clock = CLOCK, price = '102', code = 'Q-1') {
  return new graph.QuoteEvent(clock, { crosscode: code, side: 'SELL', price, quantity: 5, ticker: 'IBM' })
}

test('BookEvent: limits fold the live entries one level a price', () => {
  // An empty side has no limit and no depth to sum.
  const empty = new graph.BookEvent(CLOCK, 'IBM')
  assert.deepEqual(empty.limits('BUY'), [])
  assert.equal(empty.depth('BUY', 1), '0')

  // Two entries at one price are one limit naming both, in position
  // order; a lower price is the next limit.
  const book = empty.withOperations([
    order(CLOCK, '101', 'O-1'),
    order(CLOCK, '101', 'O-2'),
    order(CLOCK, '100', 'O-3'),
  ])
  const [first, second, third] = book.alive().map((entry) => entry.curruuid)
  assert.deepEqual(book.limits('BUY'), [
    { price: '101', quantity: '20', uuids: [first, second], tradable: true },
    { price: '100', quantity: '10', uuids: [third], tradable: true },
  ])
  // A side is named by its stored name, any spelling `Side` reads, or its
  // code - what `Side.BUY` holds.
  assert.deepEqual(book.limits(Side.BUY), book.limits('BUY'))
  assert.deepEqual(book.limits('buy'), book.limits('BUY'))
  assert.deepEqual(book.limits('SELL'), [])
  assert.equal(book.depth('BUY', 0), '0')
  assert.equal(book.depth('BUY', 1), '20')
  assert.equal(book.depth('BUY', 2), '30')
  assert.equal(book.depth('BUY', 9), '30')

  // An order stating no price - a market order - rests at the one unpriced
  // limit, after every priced one, and the best price is still the first
  // priced limit's.
  const market = new graph.OrderEvent(CLOCK, { crosscode: 'M-1', side: 'BUY', quantity: 7, ticker: 'IBM' })
  const priced = empty.withOperations([order(), market])
  const uuids = priced.alive().map((entry) => entry.curruuid)
  assert.deepEqual(priced.limits('BUY'), [
    { price: '101', quantity: '10', uuids: [uuids[0]], tradable: true },
    { price: null, quantity: '7', uuids: [uuids[1]], tradable: true },
  ])
  assert.equal(priced.bestPrice('BUY'), '101')
  assert.equal(priced.depth('BUY', 2), '17')

  // A level count is a whole number of at most 2^53, a side one `Side`
  // names.
  assert.throws(() => book.depth('BUY', -1), /levels must be a non-negative whole number/)
  assert.throws(() => book.depth('BUY', 1.5), /levels must be a non-negative whole number/)
  assert.throws(() => book.limits('SIDEWAYS'), /side/)
  assert.throws(() => book.limits(99), /side/)
})

test('BookEvent: the best bid and ask are the best tradable levels', () => {
  const stating = (code, price, tradable) =>
    new graph.OrderEvent(CLOCK, { crosscode: code, side: 'BUY', price, quantity: 1, tradable, ticker: 'IBM' })
  const book = new graph.BookEvent(CLOCK, 'IBM').withOperations([
    stating('A', '102', false),
    stating('B', '101', true),
  ])
  assert.deepEqual(book.limits('BUY').map((limit) => [limit.price, limit.tradable]), [
    ['102', false],
    ['101', true],
  ])
  assert.equal(book.bestPrice('BUY'), '101')
  assert.equal(book.bestQuantity('BUY'), '1')
  assert.equal(book.bidpx, '101')
  assert.equal(book.bidqty, '1')
  assert.equal(book.askpx, null)
  const untradable = new graph.BookEvent(CLOCK, 'IBM').withOperations([stating('A', '102', false)])
  assert.equal(untradable.bestPrice('BUY'), null)
  assert.equal(untradable.bidpx, null)
})

test('BookEvent: locked, spread and imbalance read the two bests', () => {
  const empty = new graph.BookEvent(CLOCK, 'IBM')
  assert.equal(empty.isLocked, false)
  assert.equal(empty.spread, null)
  assert.equal(empty.imbalance(1), null)

  // Equal bests are locked, never crossed, and the spread is zero.
  const locked = empty.withOperations([order(CLOCK, '101'), quote(CLOCK, '101')])
  assert.equal(locked.isLocked, true)
  assert.equal(locked.isCrossed, false)
  assert.equal(locked.spread, '0')

  // An ordinary book: the spread is ask less bid, and the imbalance the
  // signed share of the two depths.
  const book = empty.withOperations([order(CLOCK, '101'), quote(CLOCK, '102')])
  assert.equal(book.isLocked, false)
  assert.equal(book.spread, '1')
  assert.equal(book.imbalance(1), '0.333333333333333333')
  assert.equal(book.imbalance(0), null)

  // A crossed book states a negative spread: the honest fact.
  const crossed = empty.withOperations([order(CLOCK, '103'), quote(CLOCK, '102')])
  assert.equal(crossed.isCrossed, true)
  assert.equal(crossed.spread, '-1')

  // One side alone has no spread, and all of the imbalance.
  const bid = empty.withOperations([order()])
  const ask = empty.withOperations([quote()])
  assert.equal(bid.spread, null)
  assert.equal(bid.imbalance(1), '1')
  assert.equal(ask.imbalance(1), '-1')
  assert.throws(() => book.imbalance(-1), /levels must be a non-negative whole number/)
})

test('BookEvent: an empty book', () => {
  const book = new graph.BookEvent(CLOCK, 'IBM')
  assert.equal(book.currunix, CLOCK)
  assert.equal(book.crosscode, 'IBM')
  assert.equal(book.marketdatakind, 'BOOK')
  assert.equal(book.side, 'UNKNOWN')
  assert.deepEqual(book.alive(), [])
  assert.deepEqual(book.deltas(), [])
  assert.deepEqual(book.executions(), [])
  assert.equal(book.bestPrice('BUY'), null)
  assert.equal(book.bestQuantity('SELL'), null)
  assert.equal(book.isCrossed, false)
  assert.equal(book.bboMidpoint, null)
  assert.equal(book.medianQuantity, null)
})

test('BookEvent: withOperations of an order and a quote', () => {
  const empty = new graph.BookEvent(CLOCK, 'IBM')
  const book = empty.withOperations([order(), quote()])
  assert.deepEqual(empty.alive(), []) // immutable: the verb answered a new book
  assert.equal(book.bestPrice('BUY'), '101')
  assert.equal(book.bestPrice('SELL'), '102')
  assert.equal(book.bidpx, '101')
  assert.equal(book.askpx, '102')
  assert.equal(book.isCrossed, false)
  assert.equal(book.bboMidpoint, '101.5')
  assert.equal(book.medianQuantity, '7.5')
  const alive = book.alive()
  assert.ok(alive.every((entry) => entry instanceof graph.MarketData))
  // The bid side's entries first, then the ask side's.
  assert.deepEqual(alive.map((entry) => entry.side), ['BUY', 'SELL'])
  assert.ok(alive[0].asOrderEvent() instanceof graph.OrderEvent)
  assert.equal(book.deltas().length, 2)
  assert.equal(empty.withOperations([order(CLOCK, '103'), quote()]).isCrossed, true)
})

test('BookEvent: executions and MarketData fold, read from any iterable', () => {
  const execution = new graph.ExecutionEvent(CLOCK, { crosscode: 'E-1', side: 'BUY', lastpx: '101', lastqty: 1 })
  const book = new graph.BookEvent(CLOCK, 'IBM').withOperations(new Set([new graph.MarketData(order()), execution]))
  assert.ok(book.executions().every((held) => held instanceof graph.ExecutionEvent))
  // A sided element's cross code carries its side (A17).
  assert.deepEqual(book.executions().map((held) => held.crosscode), ['BUY:E-1'])
})

test('BookEvent: refusals name the item', () => {
  const book = new graph.BookEvent(CLOCK, 'IBM')
  assert.throws(
    () => book.withOperations([order(), 1]),
    /operations\[1\]: expected MarketData or a market leaf, got number/,
  )
  assert.throws(
    () => book.withOperations([new graph.Order()]),
    /\$\.operations\[0\]\.kind: expected order_event.*got order/,
  )
  assert.throws(() => book.withOperations(5), /operations must be an iterable/)
})

test('BookEvent: equals, stableHash, toString, clone and toJSON round trip', () => {
  const book = new graph.BookEvent(CLOCK, 'IBM').withOperations([order(), quote()])
  const twin = graph.BookEvent.fromJSON(book.toJSON())
  assert.ok(twin.equals(book))
  assert.equal(twin.stableHash(), book.stableHash())
  assert.ok(twin.alive().every((entry, at) => entry.equals(book.alive()[at])))
  assert.deepEqual(twin.limits('SELL'), book.limits('SELL'))
  assert.ok(book.clone().equals(book))
  assert.equal(book.toString(), `BookEvent(${book.curruuid}, currunix=${CLOCK}, crosscode="IBM")`)
  const later = new graph.BookEvent(CLOCK + 1n, 'IBM')
  assert.ok(later.isAfter(book) && book.isBefore(later))
})

test('SnapshotEvent: snapshot copies a dated leaf', () => {
  const source = order()
  const snapshot = graph.SnapshotEvent.snapshot(source, 'S')
  assert.equal(snapshot.currunix, source.currunix)
  assert.equal(snapshot.ticker, 'IBM')
  assert.equal(snapshot.marketdatakind, 'BOOK')
  assert.equal(snapshot.book.action, 'snapshot')
  assert.equal(snapshot.book.scope, 'S')
  assert.equal(graph.SnapshotEvent.snapshot(new graph.MarketData(source)).book.scope, null)
  assert.throws(() => graph.SnapshotEvent.snapshot(new graph.Order()), /expected a dated leaf to snapshot, got order/)
  assert.throws(() => new graph.SnapshotEvent())
})

test('SnapshotEvent: equals, stableHash, toString, clone and toJSON round trip', () => {
  const snapshot = graph.SnapshotEvent.snapshot(order(), 'S')
  const twin = graph.SnapshotEvent.fromJSON(snapshot.toJSON())
  assert.ok(twin.equals(snapshot))
  assert.equal(twin.stableHash(), snapshot.stableHash())
  assert.ok(twin.book.equals(snapshot.book))
  assert.ok(snapshot.clone().equals(snapshot))
  assert.ok(snapshot.toString().startsWith(`SnapshotEvent(${snapshot.curruuid}, currunix=${CLOCK}`))
})

test('BookIterator: books over three items in order', () => {
  const execution = new graph.ExecutionEvent(CLOCK + 1n, {
    crosscode: 'O-1', side: 'BUY', lastpx: '101', lastqty: 1, ticker: 'IBM',
  })
  const walk = new graph.BookIterator([order(), new graph.MarketData(quote()), execution])
  assert.equal('global' in walk, false)
  const books = [...walk]
  assert.ok(books.every((book) => book instanceof graph.BookEvent))
  assert.deepEqual(books.map((book) => book.currunix), [CLOCK, CLOCK + 1n])
  assert.equal(books[0].bestPrice('BUY'), '101')
  assert.equal(books[0].bestPrice('SELL'), '102')
  assert.deepEqual(books[1].executions().map((held) => held.crosscode), ['BUY:O-1'])
  assert.equal(walk[Symbol.iterator](), walk)
  assert.equal('GLOBAL_SYMBOL' in graph, false)
})

test('BookIterator: an undated item is refused by name', () => {
  assert.throws(
    () => [...new graph.BookIterator([new graph.Order()])],
    /\$\.operation\.kind: expected order_event.*got order/,
  )
})

test('BookIterator: a JavaScript failure is thrown as itself', () => {
  function* items() {
    yield order()
    throw new RangeError('the source gave up')
  }
  assert.throws(() => [...new graph.BookIterator(items())], { name: 'RangeError', message: 'the source gave up' })
  assert.throws(() => [...new graph.BookIterator([1])], {
    name: 'TypeError',
    message: 'expected MarketData or a market leaf, got number',
  })
  assert.throws(() => new graph.BookIterator(5), /items must be an iterable/)
})

test('BookIterator: a JavaScript failure ends the walk where it happened', () => {
  // An order resting until `CLOCK + 10` expires there once the source ends
  // ...
  const resting = new graph.OrderEvent(CLOCK, {
    crosscode: 'O-1',
    side: 'BUY',
    price: '101',
    quantity: 10,
    ticker: 'IBM',
    exprunix: CLOCK + 10n,
  })
  assert.deepEqual(
    [...new graph.BookIterator([resting])].map((book) => book.currunix),
    [CLOCK, CLOCK + 10n],
  )
  // ... but a source that failed did not end: as Python's walk and the
  // core's, the failure crosses the walk as its own error, so nothing past
  // it - the expiry included - is read as if the stream had.
  function* items() {
    yield resting
    throw new RangeError('the source gave up')
  }
  const walk = new graph.BookIterator(items())
  assert.equal(walk.next().value.currunix, CLOCK)
  assert.throws(() => walk.next(), { name: 'RangeError', message: 'the source gave up' })
  assert.deepEqual(walk.next(), { value: undefined, done: true })
})
