'use strict'

// Books, their readings by side, snapshot controls and the book walk:
// `node/src/graph/book.rs`.

const assert = require('node:assert/strict')
const test = require('node:test')

const { Filter, Serie, Side, Term, graph } = require('yggdryl')

const CLOCK = 1_700_000_000_000_000_000n

function order(clock = CLOCK, price = '101', code = 'O-1') {
  return new graph.OrderEvent(clock, { crosscode: code, side: 'BUYS', price, quantity: 10, ticker: 'IBM', instcode: 'IBM' })
}

function quote(clock = CLOCK, price = '102', code = 'Q-1') {
  return new graph.QuoteEvent(clock, { crosscode: code, side: 'SELL', price, quantity: 5, ticker: 'IBM', instcode: 'IBM' })
}

test('BookEvent: limits fold the live entries one level a price', () => {
  // An empty side has no limit and no depth to sum.
  const empty = new graph.BookEvent(CLOCK, 'IBM')
  assert.deepEqual(empty.limits('BUYS'), [])
  assert.equal(empty.depth('BUYS', 1), '0')

  // Two entries at one price are one limit naming both, in position
  // order; a lower price is the next limit.
  const book = empty.withOperations([
    order(CLOCK, '101', 'O-1'),
    order(CLOCK, '101', 'O-2'),
    order(CLOCK, '100', 'O-3'),
  ])
  const [first, second, third] = book.alive().map((entry) => entry.uuid)
  assert.deepEqual(book.limits('BUYS'), [
    { price: '101', quantity: '20', uuids: [first, second], tradable: true },
    { price: '100', quantity: '10', uuids: [third], tradable: true },
  ])
  // A side is named by its four-letter code, any spelling `Side` reads, or its
  // code - what `Side.BUYS` holds.
  assert.deepEqual(book.limits(Side.BUYS), book.limits('BUYS'))
  assert.deepEqual(book.limits('buys'), book.limits('BUYS'))
  assert.deepEqual(book.limits('SELL'), [])
  assert.equal(book.depth('BUYS', 0), '0')
  assert.equal(book.depth('BUYS', 1), '20')
  assert.equal(book.depth('BUYS', 2), '30')
  assert.equal(book.depth('BUYS', 9), '30')

  // An order stating no price - a market order - rests at the one unpriced
  // limit, after every priced one, and the best price is still the first
  // priced limit's.
  const market = new graph.OrderEvent(CLOCK, { crosscode: 'M-1', side: 'BUYS', quantity: 7, ticker: 'IBM', instcode: 'IBM' })
  const priced = empty.withOperations([order(), market])
  const uuids = priced.alive().map((entry) => entry.uuid)
  assert.deepEqual(priced.limits('BUYS'), [
    { price: '101', quantity: '10', uuids: [uuids[0]], tradable: true },
    { price: null, quantity: '7', uuids: [uuids[1]], tradable: true },
  ])
  assert.equal(priced.bestPrice('BUYS'), '101')
  assert.equal(priced.depth('BUYS', 2), '17')

  // A level count is a whole number of at most 2^53, a side one `Side`
  // names.
  assert.throws(() => book.depth('BUYS', -1), /levels must be a non-negative whole number/)
  assert.throws(() => book.depth('BUYS', 1.5), /levels must be a non-negative whole number/)
  assert.throws(() => book.limits('SIDEWAYS'), /side/)
  assert.throws(() => book.limits(98), /side/)
  // 99 is BOTH, a side no level rests on.
  assert.deepEqual(book.limits(99), [])
})

test('BookEvent: the best bid and ask are the best tradable levels', () => {
  const stating = (code, price, tradable) =>
    new graph.OrderEvent(CLOCK, { crosscode: code, side: 'BUYS', price, quantity: 1, tradable, ticker: 'IBM', instcode: 'IBM' })
  const book = new graph.BookEvent(CLOCK, 'IBM').withOperations([
    stating('A', '102', false),
    stating('B', '101', true),
  ])
  assert.deepEqual(book.limits('BUYS').map((limit) => [limit.price, limit.tradable]), [
    ['102', false],
    ['101', true],
  ])
  assert.equal(book.bestPrice('BUYS'), '101')
  assert.equal(book.bestQuantity('BUYS'), '1')
  assert.equal(book.bidpx, '101')
  assert.equal(book.bidqty, '1')
  assert.equal(book.askpx, null)
  const untradable = new graph.BookEvent(CLOCK, 'IBM').withOperations([stating('A', '102', false)])
  assert.equal(untradable.bestPrice('BUYS'), null)
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
  assert.equal(book.transunix, CLOCK)
  assert.equal(book.crosscode, '3:0:IBM')
  assert.equal(book.marketdatakind, 'BOOK')
  // A book holds both sides.
  assert.equal(book.side, 'BOTH')
  assert.deepEqual(book.aliveOn('BOTH'), [])
  assert.deepEqual(book.alive(), [])
  assert.deepEqual(book.aliveOn('BUYS'), [])
  assert.deepEqual([book.delta(), book.events()], [[], []])
  assert.deepEqual(
    [book.ordlive(), book.orddelta(), book.quotes(), book.executions(), book.controls()],
    [[], [], [], [], []],
  )
  assert.equal(book.isComplete, true, 'a book a caller builds holds its sides')
  assert.equal(book.bestPrice('BUYS'), null)
  assert.equal(book.bestQuantity('SELL'), null)
  assert.equal(book.isCrossed, false)
  assert.equal(book.bboMidpoint, null)
  assert.equal(book.medianQuantity, null)
})

test('BookEvent: withOperations of an order and a quote', () => {
  const empty = new graph.BookEvent(CLOCK, 'IBM')
  const book = empty.withOperations([order(), quote()])
  assert.deepEqual(empty.alive(), []) // immutable: the verb answered a new book
  assert.equal(book.bestPrice('BUYS'), '101')
  assert.equal(book.bestPrice('SELL'), '102')
  assert.equal(book.bidpx, '101')
  assert.equal(book.askpx, '102')
  assert.equal(book.isCrossed, false)
  assert.equal(book.bboMidpoint, '101.5')
  assert.equal(book.medianQuantity, '7.5')
  const alive = book.alive()
  assert.ok(alive.every((entry) => entry instanceof graph.MarketData))
  // The bid side's entries first, then the ask side's.
  assert.deepEqual(alive.map((entry) => entry.side), ['BUYS', 'SELL'])
  assert.ok(alive[0].asOrderEvent() instanceof graph.OrderEvent)
  // Both are the book's delta; it recorded no other event.
  assert.equal(book.delta().length, 2)
  assert.deepEqual(book.events(), [])
  assert.equal(empty.withOperations([order(CLOCK, '103'), quote()]).isCrossed, true)
})

test('BookEvent: MarketData folds, read from any iterable, and an execution is recorded', () => {
  const book = new graph.BookEvent(CLOCK, 'IBM').withOperations(new Set([new graph.MarketData(order())]))
  // The stored cross code is the kind, the side, then the base.
  assert.deepEqual(book.delta().map((held) => held.crosscode), ['10:1:O-1'])
  // A fill moves a book through its order's report: an execution alone
  // rests on no side and moves none, and stands among the events of its
  // instant, its delta empty.
  const execution = new graph.ExecutionEvent(CLOCK + 10n, {
    crosscode: 'E-1', side: 'BUYS', price: '101', quantity: 10, lastpx: '101', lastqty: 1, ticker: 'IBM', instcode: 'IBM',
  })
  const recorded = book.withOperations([execution])
  assert.equal(recorded.transunix, CLOCK + 10n)
  assert.deepEqual(recorded.delta(), [])
  assert.deepEqual(recorded.events().map((held) => held.crosscode), ['8:1:E-1'])
  assert.deepEqual(recorded.alive().map((held) => held.crosscode), ['10:1:O-1'])
  // Read by kind, the execution is an `ExecutionEvent` among the events.
  assert.ok(recorded.executions()[0] instanceof graph.ExecutionEvent)
  assert.deepEqual(recorded.executions().map((held) => held.crosscode), ['8:1:E-1'])
  assert.deepEqual(recorded.controls(), [])
  // Beside an order, the order is the delta and the execution an event.
  const later = book.withOperations([
    execution,
    new graph.OrderEvent(CLOCK + 10n, { crosscode: 'O-2', side: 'SELL', price: '102', quantity: 3, ticker: 'IBM', instcode: 'IBM' }),
  ])
  assert.equal(later.transunix, CLOCK + 10n)
  assert.deepEqual(later.delta().map((held) => held.crosscode), ['10:2:O-2'])
  assert.deepEqual(later.events().map((held) => held.crosscode), ['8:1:E-1'])
  assert.deepEqual(later.orddelta().map((held) => held.crosscode), ['10:2:O-2'])
  assert.equal(later.alive().length, 2)
  assert.equal(later.execunix, null)
})

test('BookEvent: ordlive, orddelta, quotes and executions read the entries by kind', () => {
  const book = new graph.BookEvent(CLOCK, 'IBM').withOperations([
    order(CLOCK, '101', 'B-1'),
    order(CLOCK, '100', 'B-2'),
    quote(),
  ])
  const later = book.withOperations([
    new graph.OrderEvent(CLOCK + 1n, {
      crosscode: 'B-1', side: 'BUYS', price: '101', quantity: 10, ticker: 'IBM', instcode: 'IBM', state: 'CANCELED',
    }),
    new graph.ExecutionEvent(CLOCK + 1n, { crosscode: 'E-1', side: 'BUYS', lastpx: '100', lastqty: 1, ticker: 'IBM', instcode: 'IBM' }),
    quote(CLOCK + 1n, '103', 'Q-2'),
  ])
  const codes = (entries) => entries.map((entry) => entry.crosscode)
  assert.ok([...later.ordlive(), ...later.orddelta()].every((entry) => entry instanceof graph.OrderEvent))
  // Resting: the orders alive now. Changed: the orders the instant applied.
  assert.deepEqual(codes(later.ordlive()), ['10:1:B-2'])
  assert.deepEqual(later.orddelta().map((entry) => [entry.crosscode, entry.state]), [['10:1:B-1', 'CANCELED']])
  assert.ok(later.quotes()[0] instanceof graph.QuoteEvent)
  assert.deepEqual(codes(later.quotes()), ['14:0:Q-2'])
  assert.deepEqual(codes(later.executions()), ['8:1:E-1'])
  // Q-1 still rests: it is alive, not in this instant's delta.
  assert.ok(codes(later.alive()).includes('14:0:Q-1'))
  // The orders and quotes partition the delta, the executions and the
  // controls the events; neither holds anything else.
  assert.equal(later.orddelta().length + later.quotes().length, later.delta().length)
  assert.equal(later.executions().length + later.controls().length, later.events().length)
  assert.deepEqual(codes(later.delta()), ['10:1:B-1', '14:0:Q-2'])
  assert.deepEqual(codes(later.events()), ['8:1:E-1'])
  // A delta book states its changed orders and no resting one.
  const books = [...new graph.BookIterator([order(), order(CLOCK + 1n, '100', 'B-2')])]
  assert.deepEqual([books[1].ordlive(), codes(books[1].orddelta())], [[], ['10:1:B-2']])
  const rebuilt = books[1].withPrevious(books[0].withPrevious(graph.BookEvent.keyed(CLOCK, 'IBM')))
  assert.deepEqual(codes(rebuilt.ordlive()), ['10:1:O-1', '10:1:B-2'])
})

test('BookEvent: aliveOn reads one side best first, and alive the bids then the asks', () => {
  const stating = (code, side, price, quantity) =>
    new graph.OrderEvent(CLOCK, { crosscode: code, side, price, quantity, ticker: 'IBM', instcode: 'IBM' })
  const book = new graph.BookEvent(CLOCK, 'IBM').withOperations([
    stating('B-1', 'BUYS', '100', 1),
    stating('B-M', 'BUYS', undefined, 3),
    stating('B-2', 'BUYS', '101', 2),
    stating('A-1', 'SELL', '102', 1),
  ])
  const codes = (entries) => entries.map((entry) => entry.crosscode)
  assert.ok(book.aliveOn('BUYS').every((entry) => entry instanceof graph.MarketData))
  assert.deepEqual(codes(book.aliveOn('BUYS')), ['10:1:B-2', '10:1:B-1', '10:1:B-M'])
  assert.deepEqual(codes(book.aliveOn(Side.SELL)), ['10:2:A-1'])
  assert.deepEqual(book.aliveOn('UKNW'), [])
  assert.deepEqual(codes(book.alive()), [...codes(book.aliveOn('BUYS')), ...codes(book.aliveOn('SELL'))])
  // The delta is the four orders, in the order applied.
  assert.deepEqual(codes(book.delta()), ['10:1:B-1', '10:1:B-M', '10:1:B-2', '10:2:A-1'])
  assert.throws(() => book.aliveOn('SIDEWAYS'), /side/)
})

test('BookEvent: keyed is the empty base a delta book rebuilds over', () => {
  const base = graph.BookEvent.keyed(CLOCK, 'CH0012214059')
  assert.ok(base instanceof graph.BookEvent)
  assert.equal(base.crosscode, '3:0:CH0012214059')
  assert.equal(base.ticker, null, 'a keyed book states no ticker until an input does')
  assert.equal(base.isincode, null)
  assert.equal(base.isComplete, true)
  assert.deepEqual(base.alive(), [])
  assert.equal(graph.BookEvent.keyed(1, 'IBM').transunix, 1n)
  assert.throws(() => graph.BookEvent.keyed(1.5, 'IBM'), /transunix/)

  // With no grid, a walk emits every book as a delta book, beside the top
  // of book it settled on; the first follows no book.
  const books = [...new graph.BookIterator([order(CLOCK, '99', 'B-1'), order(CLOCK + 1n, '100', 'B-2')])]
  assert.deepEqual(books.map((book) => book.isComplete), [false, false])
  assert.equal(books[0].prevuuid, null)
  assert.deepEqual(books[1].alive(), [])
  assert.deepEqual(books[1].aliveOn('BUYS'), [])
  assert.deepEqual(books[1].limits('BUYS'), [])
  assert.equal(books[1].delta().length, 1)
  assert.equal(books[1].events().length, 0)
  assert.equal(books[1].bestPrice('BUYS'), '100')
  // A delta book takes no operations until it is complete.
  assert.throws(() => books[1].withOperations([order(CLOCK + 1n, '98', 'B-3')]), /\$\.alive: a delta book takes no operations/)
  // The first is whole over the empty book a walk starts from, and the next
  // over it, each under its own identity.
  const first = books[0].withPrevious(graph.BookEvent.keyed(CLOCK, 'IBM'))
  const rebuilt = books[1].withPrevious(first)
  assert.ok(first.isComplete && rebuilt.isComplete)
  assert.equal(rebuilt.alive().length, 2)
  assert.equal(rebuilt.uuid, books[1].uuid)
  assert.deepEqual(rebuilt.limits('BUYS').map((limit) => limit.price), ['100', '99'])
})

test('BookEvent: refusals name the item', () => {
  const book = new graph.BookEvent(CLOCK, 'IBM')
  assert.throws(
    () => book.withOperations([order(), 1]),
    /operations\[1\]: expected MarketData, a market leaf or a FixMsg, got number/,
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
  assert.equal(book.toString(), `BookEvent(${book.uuid}, transunix=${CLOCK}, crosscode="3:0:IBM")`)
  const later = new graph.BookEvent(CLOCK + 1n, 'IBM')
  assert.ok(later.isAfter(book) && book.isBefore(later))
})

test('SnapshotEvent: snapshot copies a dated leaf', () => {
  const source = order()
  const snapshot = graph.SnapshotEvent.snapshot(source, 'S')
  assert.equal(snapshot.transunix, source.transunix)
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
  assert.ok(snapshot.toString().startsWith(`SnapshotEvent(${snapshot.uuid}, transunix=${CLOCK}`))
})

test('BookIterator: books over three items in order', () => {
  const execution = new graph.ExecutionEvent(CLOCK + 1n, {
    crosscode: 'O-1', side: 'BUYS', lastpx: '101', lastqty: 1, ticker: 'IBM', instcode: 'IBM',
  })
  const walk = new graph.BookIterator([order(), new graph.MarketData(quote()), execution])
  assert.equal('global' in walk, false)
  const books = [...walk]
  assert.ok(books.every((book) => book instanceof graph.BookEvent))
  // The walk records the execution where it pulls it, so the instant only
  // it reached emits an event-only book; with no grid every book is a
  // delta book.
  assert.deepEqual(books.map((book) => book.transunix), [CLOCK, CLOCK + 1n])
  assert.ok(books.every((book) => !book.isComplete))
  assert.deepEqual(books[0].delta().map((held) => held.crosscode), ['10:1:O-1', '14:0:Q-1'])
  assert.equal(books[0].bestPrice('BUYS'), '101')
  assert.equal(books[0].bestPrice('SELL'), '102')
  assert.deepEqual(books[1].delta(), [])
  assert.deepEqual(books[1].events().map((held) => held.crosscode), ['8:1:O-1'])
  assert.equal(books[1].bestPrice('BUYS'), '101')
  // A positive grid emits the book complete at each tick it holds an
  // entry, and the execution's instant between ticks as a delta book.
  const ticked = [...new graph.BookIterator([order(), new graph.MarketData(quote()), execution], 1)]
  assert.deepEqual(ticked.map((book) => book.isComplete), [true, false])
  assert.equal(ticked[0].alive().length, 2)
  assert.equal(walk[Symbol.iterator](), walk)
  assert.equal('GLOBAL_SYMBOL' in graph, false)
})

test('BookIterator: a filter over the marketdata row narrows what the books fold', () => {
  const stating = (code, side, clock) =>
    new graph.OrderEvent(clock, { crosscode: code, side, price: '100', quantity: 1, ticker: 'ACME', instcode: 'ACME' })
  const inputs = () => [stating('B-1', 'BUYS', 1), stating('A-1', 'SELL', 2)]
  // The text of a predicate, a `Filter` or a `Term`: the ask never reached a
  // book, so its instant emitted none.
  for (const filter of ["side = 'BUYS'", new Filter("side = 'BUYS'"), Term.parse("side = 'BUYS'")]) {
    const books = [...new graph.BookIterator(inputs(), 0, filter)]
    assert.equal(books.length, 1, String(filter))
    assert.deepEqual(books[0].delta().map((held) => held.crosscode), ['10:1:B-1'])
  }
  // Not given, every booked input is kept.
  assert.equal([...new graph.BookIterator(inputs(), 0, undefined)].length, 2)
  assert.equal([...new graph.BookIterator(inputs(), 0)].length, 2)
  // An execution is recorded: a filter keeping it alone folds its book.
  const execution = new graph.ExecutionEvent(3, { crosscode: 'E-1', side: 'BUYS', lastpx: '100', lastqty: 1, ticker: 'ACME', instcode: 'ACME' })
  assert.equal([...new graph.BookIterator([...inputs(), execution], 0, "marketdatakind = 'EXEC'")].length, 1)
  // A column the row does not carry is refused where the filter is bound.
  assert.throws(() => new graph.BookIterator([], 0, 'nope = 1'), /nope/)
})

test('BookIterator: an instant recording only an execution emits an event-only book', () => {
  const stating = (code, side, price, quantity) =>
    new graph.OrderEvent(CLOCK, { crosscode: code, side, price, quantity, ticker: 'IBM', instcode: 'IBM' })
  const fill = new graph.ExecutionEvent(CLOCK + 1n, {
    crosscode: 'E-1', side: 'BUYS', price: '100', lastpx: '100', lastqty: 1, ticker: 'IBM', instcode: 'IBM', state: 'FILLED',
  })
  const books = [...new graph.BookIterator([stating('B-1', 'BUYS', '100', 2), stating('A-1', 'SELL', '102', 3), fill])]
  assert.deepEqual(books.map((book) => book.transunix), [CLOCK, CLOCK + 1n], "the execution's instant emits a book")
  // A delta book whose delta is empty and whose events hold the execution:
  // no control, no snapshot instant.
  const eventOnly = books[1]
  assert.equal(eventOnly.isComplete, false)
  assert.equal(eventOnly.snapunix, null)
  assert.deepEqual(eventOnly.delta(), [])
  assert.deepEqual(eventOnly.events().map((held) => held.crosscode), ['8:1:E-1'])
  assert.deepEqual(eventOnly.executions().map((held) => held.crosscode), ['8:1:E-1'])
  assert.deepEqual(eventOnly.controls(), [])
  assert.equal(eventOnly.prevuuid, books[0].uuid)
  // Its top of book is the one the execution left standing.
  assert.deepEqual([eventOnly.bidpx, eventOnly.askpx], ['100', '102'])
  // Rebuilt over the complete book before it, nothing is replayed: the
  // sides and the limits are the previous book's, under its own identity.
  const previous = books[0].withPrevious(graph.BookEvent.keyed(CLOCK, 'IBM'))
  const rebuilt = eventOnly.withPrevious(previous)
  assert.equal(rebuilt.isComplete, true)
  assert.equal(rebuilt.uuid, eventOnly.uuid)
  assert.deepEqual(rebuilt.alive().map((held) => held.crosscode), previous.alive().map((held) => held.crosscode))
  assert.deepEqual(rebuilt.limits('BUYS'), previous.limits('BUYS'))
  assert.deepEqual(rebuilt.limits('SELL'), previous.limits('SELL'))
  assert.deepEqual(rebuilt.events().map((held) => held.crosscode), ['8:1:E-1'])
  // Laid out as a row and read back, it is the book it is, never a control.
  const [back] = [...graph.MarketData.fromArrowReader(graph.MarketData.arrowReader([eventOnly]))]
  const read = back.asBookEvent()
  assert.ok(read instanceof graph.BookEvent)
  assert.ok(read.equals(eventOnly))
  assert.deepEqual([read.delta(), read.events().map((held) => held.crosscode)], [[], ['8:1:E-1']])
})

test('BookIterator: an empty snapshot on an empty book emits an empty complete book holding the control', () => {
  // A full refresh stating no entry changes no membership, yet it is
  // recorded: the book is complete, its events the control, its snapshot
  // instant the control's.
  const reset = new graph.OrderEvent(CLOCK, { crosscode: 'RESET', ticker: 'IBM', instcode: 'IBM', state: 'NEW', snapunix: CLOCK })
  const control = graph.SnapshotEvent.snapshot(reset)
  const books = [...new graph.BookIterator([control])]
  assert.equal(books.length, 1)
  const [book] = books
  assert.equal(book.isComplete, true)
  assert.equal(book.transunix, CLOCK)
  assert.equal(book.snapunix, CLOCK)
  assert.equal(book.ticker, 'IBM')
  assert.deepEqual([book.alive(), book.delta(), book.executions()], [[], [], []])
  assert.equal(book.events().length, 1)
  const controls = book.controls()
  assert.ok(controls[0] instanceof graph.SnapshotEvent)
  assert.deepEqual(controls.map((held) => held.uuid), [control.uuid])
  assert.equal(controls[0].book.action, 'snapshot')
  assert.deepEqual([book.limits('BUYS'), book.limits('SELL')], [[], []])
})

test('MarketData: deltaSerie and eventsSerie read a table of books by kind', () => {
  const fill = new graph.ExecutionEvent(CLOCK + 2n, {
    crosscode: 'E-1', side: 'BUYS', lastqty: 100, ticker: 'IBM', instcode: 'IBM',
  })
  const stating = (clock, code, side) =>
    new graph.OrderEvent(clock, { crosscode: code, side, price: '100', quantity: 1, ticker: 'IBM', instcode: 'IBM' })
  const inputs = [stating(CLOCK, 'B-1', 'BUYS'), stating(CLOCK + 1n, 'A-1', 'SELL'), fill]
  // Three books, one per instant - the fill's an event-only delta book -
  // laid out as rows once and held: what a table of books holds.
  const rows = () =>
    Serie.fromArrowReader(graph.MarketData.arrowReader(new graph.BookIterator(inputs)))
  assert.equal(rows().length, 3)
  const kinds = (serie) =>
    [...graph.MarketData.fromArrowReader(serie.intoArrowReader())].map((data) => [data.marketdatakind, data.crosscode])
  // The two orders are the books' delta, the fill their events.
  assert.deepEqual(kinds(graph.MarketData.deltaSerie(rows())), [['ORDR', '10:1:B-1'], ['ORDR', '10:2:A-1']])
  assert.equal(kinds(graph.MarketData.deltaSerie(rows(), 'ORDR')).length, 2)
  assert.deepEqual(kinds(graph.MarketData.deltaSerie(rows(), 'QUOT')), [])
  assert.deepEqual(kinds(graph.MarketData.eventsSerie(rows())), [['EXEC', '8:1:E-1']])
  assert.deepEqual(kinds(graph.MarketData.eventsSerie(rows(), 'EXEC')), [['EXEC', '8:1:E-1']])
  assert.deepEqual(kinds(graph.MarketData.eventsSerie(rows(), 'BOOK')), [])
  // An empty full refresh's control is its book's event, read by `BOOK`.
  const reset = new graph.OrderEvent(CLOCK, { crosscode: 'RESET', ticker: 'IBM', instcode: 'IBM', state: 'NEW', snapunix: CLOCK })
  const controlled = Serie.fromArrowReader(
    graph.MarketData.arrowReader(new graph.BookIterator([graph.SnapshotEvent.snapshot(reset)])),
  )
  assert.deepEqual(kinds(graph.MarketData.eventsSerie(controlled, 'BOOK')).map(([kind]) => kind), ['BOOK'])
  // A kind is read through the core vocabulary.
  assert.throws(() => graph.MarketData.deltaSerie(rows(), 'NOPE'), /NOPE/)
})

test('BookIterator: an undated item is refused by name', () => {
  // Stating its instcode, so it is admitted and refused by its kind rather
  // than pruned as code-less.
  assert.throws(
    () => [...new graph.BookIterator([new graph.Order({ instcode: 'IBM' })])],
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
    message: 'expected MarketData, a market leaf or a FixMsg, got number',
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
    ticker: 'IBM', instcode: 'IBM',
    exprunix: CLOCK + 10n,
  })
  assert.deepEqual(
    [...new graph.BookIterator([resting])].map((book) => book.transunix),
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
  assert.equal(walk.next().value.transunix, CLOCK)
  assert.throws(() => walk.next(), { name: 'RangeError', message: 'the source gave up' })
  assert.deepEqual(walk.next(), { value: undefined, done: true })
})
