'use strict'

// `graph.TradeEvent`: `node/src/graph/trade.rs`.

const assert = require('node:assert/strict')
const test = require('node:test')

const { graph } = require('yggdryl')

const CLOCK = 1_700_000_000_000_000_000n

function fill(code, side, quantity, clock = CLOCK) {
  return new graph.ExecutionEvent(clock, { crosscode: code, side, lastpx: '101.25', lastqty: quantity, ticker: 'ACME' })
}

function trade() {
  const root = new graph.ExecutionEvent(CLOCK, { crosscode: 'T-1', ticker: 'ACME', lastpx: '101.25', lastqty: 10 })
  return graph.TradeEvent.fromParts(root, [fill('SELL-1', 'SELL', 6), fill('BUY-1', 'BUY', 4)])
}

test('fromParts of two executions', () => {
  const made = trade()
  assert.equal(made.crosscode, 'T-1')
  assert.equal(made.currunix, CLOCK)
  assert.equal(made.ticker, 'ACME')
  assert.equal(made.lastqty, '10')
  const { executions } = made
  assert.ok(executions.every((execution) => execution instanceof graph.ExecutionEvent))
  // In canonical side order, whatever order they were handed over in.
  assert.deepEqual(executions.map((execution) => execution.side), ['BUY', 'SELL'])
  assert.deepEqual(executions.map((execution) => execution.crosscode).sort(), ['BUY-1', 'SELL-1'])
  assert.equal(made.isExecution, true)
})

test('any dated operation or MarketData roots a trade', () => {
  const root = new graph.OrderEvent(CLOCK, { crosscode: 'T-1', ticker: 'ACME' })
  const byLeaf = graph.TradeEvent.fromParts(root, [fill('B', 'BUY', 1)])
  const byData = graph.TradeEvent.fromParts(new graph.MarketData(root), [fill('B', 'BUY', 1)])
  assert.ok(byLeaf.equals(byData))
  assert.equal(byLeaf.crosscode, 'T-1')
})

test('refusals name what was wrong', () => {
  assert.throws(
    () => graph.TradeEvent.fromParts(new graph.Order(), []),
    /expected a dated operation as the trade's root, got order/,
  )
  assert.throws(
    () => graph.TradeEvent.fromParts(new graph.ExecutionEvent(CLOCK + 1n), [fill('B', 'BUY', 1)]),
    /executions\[0\]\.currunix: expected the trade timestamp/,
  )
  assert.throws(() => graph.TradeEvent.fromParts(new graph.ExecutionEvent(CLOCK), []), /at least one execution/)
  assert.throws(() => graph.TradeEvent.fromParts(new graph.ExecutionEvent(CLOCK), [new graph.OrderEvent(CLOCK)]))
  assert.throws(() => new graph.TradeEvent())
})

test('the verbs answer new trades', () => {
  const first = trade()
  const later = graph.TradeEvent.fromParts(
    new graph.ExecutionEvent(CLOCK + 1n, { crosscode: 'T-1', ticker: 'ACME' }),
    [fill('BUY-1', 'BUY', 4, CLOCK + 1n)],
  )
  const followed = later.withPrevious(first)
  assert.equal(followed.prevuuid, first.curruuid)
  assert.equal(later.prevuuid, null)
  assert.ok(later.isAfter(first) && first.isBefore(later))
  // A trade folds another statement of itself into the same trade.
  const merged = first.mergeWith(first)
  assert.ok(merged === null || merged instanceof graph.TradeEvent)
  assert.ok(first.restating(first).equals(first))
})

test('equals, stableHash, toString, clone and toJSON round trip', () => {
  const made = trade()
  const twin = graph.TradeEvent.fromJSON(made.toJSON())
  assert.ok(twin.equals(made))
  assert.equal(twin.stableHash(), made.stableHash())
  assert.ok(twin.executions.every((execution, at) => execution.equals(made.executions[at])))
  assert.ok(made.clone().equals(made))
  assert.equal(made.toString(), `TradeEvent(${made.curruuid}, currunix=${CLOCK}, crosscode="T-1")`)
})
