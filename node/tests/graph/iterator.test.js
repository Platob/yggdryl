'use strict'

// `graph.EventIterator`: `node/src/graph/iterator.rs`.

const assert = require('node:assert/strict')
const test = require('node:test')

const { graph } = require('yggdryl')

const CLOCK = 1_700_000_000_000_000_000n

function order(clock, state = 'NEW', facts = {}) {
  return new graph.OrderEvent(clock, { crosscode: 'O-1', side: 'BUYS', price: '101', quantity: 10, state, ...facts })
}

test('an order chains to the live order it follows', () => {
  const first = order(CLOCK)
  const second = order(CLOCK + 1n, 'REPLACED', { leavesqty: undefined })
  const walked = [...new graph.EventIterator([first, second])]
  assert.ok(walked.every((data) => data instanceof graph.MarketData))
  const head = walked[0].asOrderEvent()
  const tail = walked[1].asOrderEvent()
  // The head is the order itself, the walk stating when its chain began.
  assert.equal(head.curruuid, first.curruuid)
  assert.equal(head.creaunix, CLOCK)
  assert.equal(head.seqnum, 0)
  assert.equal(head.prevuuid, null)
  assert.equal(tail.prevuuid, first.curruuid)
  assert.equal(tail.prevunix, CLOCK)
  // A later instant keeps its own place.
  assert.equal(tail.seqnum, 0)
})

test('an execution and every other leaf walk through', () => {
  const first = order(CLOCK)
  // A millisecond later: two instants in one millisecond share an identity.
  const execution = new graph.ExecutionEvent(CLOCK + 1_000_000n, { crosscode: 'O-1', side: 'BUYS', lastpx: '101', lastqty: 10 })
  const book = new graph.BookEvent(CLOCK + 2_000_000n, 'IBM')
  const walked = [...new graph.EventIterator([first, execution, book, new graph.Order({ crosscode: 'O-1' })])]
  assert.deepEqual(walked.map((data) => data.kind), ['order_event', 'execution_event', 'book_event', 'order'])
  // A book is yielded unchanged, in place.
  assert.ok(walked[2].asBookEvent().equals(book))
  assert.ok(walked[3].asOrder().equals(new graph.Order({ crosscode: 'O-1' })))
  // A chain matches within one market data kind: the execution shares the
  // order's base but is a chain of its own, following nothing. The stored
  // cross codes differ (`8:1:O-1`, `10:1:O-1`), so the cross identities do.
  const fill = walked[1].asExecutionEvent()
  assert.equal(fill.crosscode, '8:1:O-1')
  assert.notEqual(fill.crossuuid, first.crossuuid)
  assert.equal(fill.crossuuid, execution.crossuuid)
  assert.equal(fill.prevuuid, null)
  assert.equal(fill.creaunix, CLOCK + 1_000_000n)
  assert.equal(fill.seqnum, 0)
})

test('unsorted items are sorted first', () => {
  const first = order(CLOCK)
  const second = order(CLOCK + 1n, 'REPLACED')
  const walked = [...new graph.EventIterator([second, first], false)]
  assert.deepEqual(walked.map((data) => data.asOrderEvent().currunix), [CLOCK, CLOCK + 1n])
})

test('alive and the snapshot grid', () => {
  const walk = new graph.EventIterator([order(CLOCK, 'NEW', { exprunix: CLOCK + 10n })], true, 5)
  assert.equal(walk.snapshotNs, 5n)
  const walked = [...walk].map((data) => data.asOrderEvent())
  // Each view is dated at its tick and keeps the instant the order was stated at.
  assert.deepEqual(
    walked.map((event) => [event.currunix, event.snapunix]),
    [[CLOCK, null], [CLOCK, CLOCK], [CLOCK + 5n, CLOCK], [CLOCK + 10n, null]],
  )
  assert.equal(walked.at(-1).state, 'EXPIRED')
  assert.deepEqual(walk.alive(), [])
  const live = new graph.EventIterator([order(CLOCK)])
  assert.equal(live.snapshotNs, null)
  assert.equal([...live].length, 1)
  // The stored cross code is the kind, the side, then the base.
  assert.deepEqual(live.alive().map((data) => data.crosscode), ['10:1:O-1'])
  assert.ok(live.alive().every((data) => data instanceof graph.MarketData))
})

test('a snapshot view is the live event as of its tick', () => {
  // A 5 ms grid: two ticks a millisecond apart derive two identities.
  const source = order(CLOCK, 'NEW', { exprunix: CLOCK + 10_000_000n })
  const walked = [...new graph.EventIterator([source], true, 5_000_000n)].map((data) => data.asOrderEvent())
  assert.deepEqual(walked.map((event) => event.snapunix), [null, CLOCK, CLOCK, null])
  const [live] = walked
  const views = walked.filter((event) => event.snapunix !== null)
  assert.deepEqual(views.map((view) => view.currunix), [CLOCK, CLOCK + 5_000_000n])
  for (const view of views) {
    // Dated at its tick, so it has the identity that tick derives, and
    // keeping the instant the live event was stated at ...
    assert.equal(view.snapunix, live.currunix)
    assert.equal(view.curruuid === live.curruuid, view.currunix === live.currunix)
    // ... while its content, its place and its cross element are the live event's.
    assert.equal(view.currhashcode, live.currhashcode)
    assert.equal(view.seqnum, live.seqnum)
    assert.equal(view.prevuuid, live.prevuuid)
    assert.equal(view.crossuuid, live.crossuuid)
  }
  assert.notEqual(views[1].curruuid, live.curruuid)
  // A view does not advance the chain: the expiry follows the live event.
  assert.equal(walked.at(-1).prevuuid, live.curruuid)
})

test('a JavaScript failure is thrown as itself', () => {
  function* items() {
    yield order(CLOCK)
    throw new RangeError('the source gave up')
  }
  const walk = new graph.EventIterator(items())
  assert.equal(walk.next().value.kind, 'order_event')
  assert.throws(() => walk.next(), { name: 'RangeError', message: 'the source gave up' })
  assert.deepEqual(walk.next(), { value: undefined, done: true })
  // Unsorted, the whole source is read at once, so the failure is too.
  assert.throws(() => new graph.EventIterator(items(), false), { name: 'RangeError' })
  assert.throws(() => [...new graph.EventIterator([1])], {
    name: 'TypeError',
    message: 'expected MarketData, a market leaf or a FixMsg, got number',
  })
  assert.throws(() => new graph.EventIterator(5), /items must be an iterable/)
  assert.throws(() => graph.EventIterator([]), /cannot be invoked without 'new'/)
})

test('the walk is its own iterator', () => {
  const walk = new graph.EventIterator([])
  assert.equal(walk[Symbol.iterator](), walk)
  assert.ok(walk instanceof graph.EventIterator)
})
