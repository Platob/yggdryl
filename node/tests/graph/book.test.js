'use strict'

// Books, their sides, snapshot controls and the book walk:
// `node/src/graph/book.rs`.

const assert = require('node:assert/strict')
const test = require('node:test')

const { graph } = require('yggdryl')

const CLOCK = 1_700_000_000_000_000_000n

function order(clock = CLOCK, price = '101', code = 'O-1') {
  return new graph.OrderEvent(clock, { crosscode: code, side: 'BUY', price, quantity: 10, ticker: 'IBM' })
}

function quote(clock = CLOCK, price = '102', code = 'Q-1') {
  return new graph.QuoteEvent(clock, { crosscode: code, side: 'SELL', price, quantity: 5, ticker: 'IBM' })
}

test('SnapshotPartition: slots, order and the value protocols', () => {
  const partition = new graph.SnapshotPartition({ scope: 'S', symbol: 'IBM' })
  assert.equal(partition.scope, 'S')
  assert.equal(partition.symbol, 'IBM')
  assert.equal(new graph.SnapshotPartition({ scope: 'S' }).symbol, null)
  assert.equal(new graph.SnapshotPartition({ scope: 'S', symbol: null }).symbol, null)
  assert.equal(new graph.SnapshotPartition({ scope: 'A' }).compare(new graph.SnapshotPartition({ scope: 'B' })), -1)
  assert.ok(new graph.SnapshotPartition(partition.toJSON()).equals(partition))
  assert.equal(new graph.SnapshotPartition(partition.toJSON()).stableHash(), partition.stableHash())
  assert.notEqual(new graph.SnapshotPartition({ scope: 'S' }).stableHash(), partition.stableHash())
  assert.ok(partition.clone().equals(partition))
  assert.equal(partition.toString(), 'SnapshotPartition(scope="S", symbol="IBM")')
  assert.equal(new graph.SnapshotPartition({ scope: 'S' }).toString(), 'SnapshotPartition(scope="S", symbol=null)')
})

test('BookSide: an empty side', () => {
  const side = new graph.BookSide('BUY')
  assert.equal(side.side, 'BUY')
  assert.equal(new graph.BookSide('sell').side, 'SELL')
  assert.equal(side.length, 0)
  assert.equal(side.isEmpty, true)
  assert.equal(side.bestPrice, null)
  assert.equal(side.bestQuantity, null)
  assert.deepEqual(side.live, [])
  assert.deepEqual(side.deltas, [])
})

test('BookSide: withOperation answers a new side', () => {
  const empty = new graph.BookSide('BUY')
  const side = empty.withOperation(order()).withOperation(new graph.MarketData(order(CLOCK, '100', 'O-2')))
  assert.equal(empty.length, 0)
  assert.equal(side.length, 2)
  assert.equal(side.isEmpty, false)
  assert.equal(side.bestPrice, '101')
  assert.equal(side.bestQuantity, '10')
  assert.ok(side.live.every((entry) => entry instanceof graph.MarketData))
  assert.deepEqual(side.live.map((entry) => entry.kind), ['order_event', 'order_event'])
  assert.deepEqual(side.live.map((entry) => entry.crosscode), ['O-1', 'O-2'])
  assert.equal(side.deltas.length, 2)
})

test('BookSide: refusals', () => {
  assert.throws(() => new graph.BookSide('SIDEWAYS'), /side/)
  assert.throws(
    () => new graph.BookSide('BUY').withOperation(new graph.ExecutionEvent(CLOCK)),
    /expected an order or quote on a book side/,
  )
  assert.throws(() => new graph.BookSide('BUY').withOperation(1), /expected MarketData or a market leaf, got number/)
})

test('BookSide: equals, stableHash, toString, clone and toJSON round trip', () => {
  const side = new graph.BookSide('BUY').withOperation(order())
  const twin = graph.BookSide.fromJSON(side.toJSON())
  assert.ok(twin.equals(side))
  assert.equal(twin.stableHash(), side.stableHash())
  assert.ok(twin.live.every((entry, at) => entry.equals(side.live[at])))
  assert.ok(side.clone().equals(side))
  assert.ok(side.toString().startsWith(`BookSide(${side.curruuid}`))
})

test('BookEvent: an empty book', () => {
  const book = new graph.BookEvent(CLOCK, 'IBM')
  assert.equal(book.currunix, CLOCK)
  assert.equal(book.crosscode, 'IBM')
  assert.ok(book.bid instanceof graph.BookSide && book.ask instanceof graph.BookSide)
  assert.equal(book.bid.side, 'BUY')
  assert.equal(book.ask.side, 'SELL')
  assert.deepEqual(book.executions, [])
  assert.deepEqual(book.snapshotPartitions, [])
  assert.equal(book.isCrossed, false)
  assert.equal(book.bboMidpoint, null)
  assert.equal(book.medianQuantity, null)
})

test('BookEvent: withOperations of an order and a quote', () => {
  const empty = new graph.BookEvent(CLOCK, 'IBM')
  const book = empty.withOperations([order(), quote()])
  assert.equal(empty.bid.isEmpty, true) // immutable: the verb answered a new book
  assert.equal(book.bid.bestPrice, '101')
  assert.equal(book.ask.bestPrice, '102')
  assert.equal(book.isCrossed, false)
  assert.equal(book.bboMidpoint, '101.5')
  assert.equal(book.medianQuantity, '7.5')
  const { live } = book.bid
  assert.ok(live.every((entry) => entry instanceof graph.MarketData))
  assert.ok(live[0].asOrderEvent() instanceof graph.OrderEvent)
  assert.equal(empty.withOperations([order(CLOCK, '103'), quote()]).isCrossed, true)
})

test('BookEvent: executions and MarketData fold, read from any iterable', () => {
  const execution = new graph.ExecutionEvent(CLOCK, { crosscode: 'E-1', side: 'BUY', lastpx: '101', lastqty: 1 })
  const book = new graph.BookEvent(CLOCK, 'IBM').withOperations(new Set([new graph.MarketData(order()), execution]))
  assert.ok(book.executions.every((held) => held instanceof graph.ExecutionEvent))
  assert.deepEqual(book.executions.map((held) => held.crosscode), ['E-1'])
})

test('BookEvent: a snapshot control records its partition', () => {
  const snapshot = graph.SnapshotEvent.snapshot(order(), 'Symbol=IBM')
  const book = new graph.BookEvent(CLOCK, 'IBM').withOperations([snapshot])
  assert.deepEqual(book.snapshotPartitions.map((partition) => partition.scope), ['Symbol=IBM'])
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
  assert.ok(twin.bid.equals(book.bid) && twin.ask.equals(book.ask))
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
  assert.equal(walk.global, false)
  const books = [...walk]
  assert.ok(books.every((book) => book instanceof graph.BookEvent))
  assert.deepEqual(books.map((book) => book.currunix), [CLOCK, CLOCK + 1n])
  assert.equal(books[0].bid.bestPrice, '101')
  assert.equal(books[0].ask.bestPrice, '102')
  assert.deepEqual(books[1].executions.map((held) => held.crosscode), ['O-1'])
  assert.equal(walk[Symbol.iterator](), walk)
})

test('BookIterator: a global walk consolidates symbols', () => {
  const walk = new graph.BookIterator([order()], 0, true)
  assert.equal(walk.global, true)
  const [book] = walk
  assert.equal(book.crosscode, graph.GLOBAL_SYMBOL)
})

test('BookIterator: a book side item is refused by name', () => {
  assert.throws(
    () => [...new graph.BookIterator([new graph.BookSide('BUY')])],
    /\$\.operation\.kind: expected order_event.*got book_side/,
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
