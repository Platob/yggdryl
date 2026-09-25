'use strict'

// `graph.EventIterator`: `node/src/graph/iterator.rs`.

const assert = require('node:assert/strict')
const test = require('node:test')

const { graph } = require('yggdryl')

const CLOCK = 1_700_000_000_000_000_000n

function order(clock, state = 'NEW', facts = {}) {
  return new graph.OrderEvent(clock, { crosscode: 'O-1', side: 'BUY', price: '101', quantity: 10, state, ...facts })
}

test('an order chains to the live order it follows', () => {
  const first = order(CLOCK)
  const second = order(CLOCK + 1n, 'REPLACED', { leavesqty: undefined })
  const walked = [...new graph.EventIterator([first, second])]
  assert.ok(walked.every((data) => data instanceof graph.MarketData))
  const head = walked[0].asOrderEvent()
  const tail = walked[1].asOrderEvent()
  assert.ok(head.equals(first))
  assert.equal(head.seqnum, 0)
  assert.equal(head.prevuuid, null)
  assert.equal(tail.prevuuid, first.curruuid)
  assert.equal(tail.prevunix, CLOCK)
  assert.equal(tail.seqnum, 1)
})

test('an execution and every other leaf walk through', () => {
  const first = order(CLOCK)
  // A millisecond later: two instants in one millisecond share an identity.
  const execution = new graph.ExecutionEvent(CLOCK + 1_000_000n, { crosscode: 'O-1', side: 'BUY', lastpx: '101', lastqty: 10 })
  const side = new graph.BookSide('BUY')
  const walked = [...new graph.EventIterator([first, execution, side, new graph.Order({ crosscode: 'O-1' })])]
  assert.deepEqual(walked.map((data) => data.kind), ['order_event', 'execution_event', 'book_side', 'order'])
  // A book side is yielded unchanged, in place.
  assert.ok(walked[2].asBookSide().equals(side))
  assert.ok(walked[3].asOrder().equals(new graph.Order({ crosscode: 'O-1' })))
  // The execution follows the order it fills across kinds, keeping its own.
  const fill = walked[1].asExecutionEvent()
  assert.equal(fill.crossuuid, first.crossuuid)
  assert.equal(fill.prevuuid, first.curruuid)
  assert.equal(fill.seqnum, 1)
})

test('unsorted items are sorted first', () => {
  const first = order(CLOCK)
  const second = order(CLOCK + 1n, 'REPLACED')
  const walked = [...new graph.EventIterator([second, first], false)]
  assert.deepEqual(walked.map((data) => data.asOrderEvent().currunix), [CLOCK, CLOCK + 1n])
})

test('alive and the snapshot grid', () => {
  const walk = new graph.EventIterator([order(CLOCK, 'NEW', { exprtime: CLOCK + 10n })], true, 5)
  assert.equal(walk.snapshotNs, 5n)
  const walked = [...walk].map((data) => data.asOrderEvent())
  assert.deepEqual(walked.map((event) => event.snapunix), [null, CLOCK, CLOCK + 5n, null])
  assert.equal(walked.at(-1).state, '95EXPIRED')
  assert.deepEqual(walk.alive(), [])
  const live = new graph.EventIterator([order(CLOCK)])
  assert.equal(live.snapshotNs, null)
  assert.equal([...live].length, 1)
  assert.deepEqual(live.alive().map((data) => data.crosscode), ['O-1'])
  assert.ok(live.alive().every((data) => data instanceof graph.MarketData))
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
    message: 'expected MarketData or a market leaf, got number',
  })
  assert.throws(() => new graph.EventIterator(5), /items must be an iterable/)
  assert.throws(() => graph.EventIterator([]), /cannot be invoked without 'new'/)
})

test('the walk is its own iterator', () => {
  const walk = new graph.EventIterator([])
  assert.equal(walk[Symbol.iterator](), walk)
  assert.ok(walk instanceof graph.EventIterator)
})
