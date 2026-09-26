'use strict'

// The synthetic replay: `node/replay/synthetic.js`.

const assert = require('node:assert/strict')
const test = require('node:test')

const { graph } = require('yggdryl')

const { books, synthetic, SYMBOLS, T0 } = require('../../replay/synthetic.js')
const { instantOf } = require('../../replay/walk.js')

const hashes = (walked) => walked.map((book) => String(book.stableHash()))

test('the operations: two tickers, one market order, one trade, one expiry, one snapshot', () => {
  const operations = synthetic()
  assert.ok(operations.every((item) => item instanceof graph.MarketData))
  const census = {}
  for (const item of operations) census[item.kind] = (census[item.kind] ?? 0) + 1
  assert.deepEqual(census, {
    quote_event: 14,
    order_event: 5,
    execution_event: 2,
    trade_event: 1,
    snapshot_event: 1,
  })
  assert.deepEqual([...new Set(operations.map((item) => item.ticker))], SYMBOLS)

  const leaves = operations.map((item) => item.intoLeaf())
  const market = leaves.filter((leaf) => leaf instanceof graph.OrderEvent && leaf.price === null)
  assert.deepEqual(market.map((leaf) => leaf.crosscode), ['ALPHA-O-3'])

  const [trade] = leaves.filter((leaf) => leaf instanceof graph.TradeEvent)
  assert.deepEqual(trade.executions.map((fill) => [fill.side, fill.lastpx, fill.lastqty]), [
    ['BUY', '82.5', '30'],
    ['SELL', '82.5', '30'],
  ])

  const [expired] = leaves.filter((leaf) => leaf.state === 'EXPIRED')
  const [first] = leaves.filter((leaf) => leaf.crosscode === expired.crosscode && leaf !== expired)
  assert.equal(expired.prevuuid, first.curruuid)

  const [snapshot] = leaves.filter((leaf) => leaf instanceof graph.SnapshotEvent)
  assert.equal(snapshot.book.action, 'snapshot')
  assert.equal(snapshot.book.scope, 'Symbol=BETA')

  // Walk order is instant order, and two statements stand one nanosecond apart.
  const instants = operations.map(instantOf)
  assert.ok(instants.every((at, index) => index === 0 || instants[index - 1] <= at))
  assert.equal(instants[0], T0)
  assert.ok(instants.some((at, index) => index > 0 && at - instants[index - 1] === 1n))
})

test('the scenario is the same on every call', () => {
  assert.deepEqual(
    synthetic().map((item) => String(item.stableHash())),
    synthetic().map((item) => String(item.stableHash())),
  )
})

test('the per-symbol walk: eleven books, pinned by their native hashes', () => {
  // The hashes digest every event's state as its `int32` code.
  const walked = books()
  assert.deepEqual(hashes(walked), [
    '12134116927728876324',
    '1948837484246932780',
    '14748324727553988236',
    '8738699681098837376',
    '12596396684464936786',
    '2512406615155876254',
    '17117599677849437215',
    '15610935616060186366',
    '15020936496071332081',
    '16874917826057616008',
    '11226100986833652445',
  ])
  assert.deepEqual([...new Set(walked.map((book) => book.crosscode))], SYMBOLS)
  // The market order rests at the unpriced limit, last on its side.
  const alpha = walked.filter((book) => book.crosscode === 'ALPHA').at(-1)
  assert.deepEqual(alpha.bid.limits.map((limit) => limit.price), ['82', '81.5', '81', null])
  // The snapshot replaced the whole BETA book with its two quotes.
  const beta = walked.at(-1)
  assert.deepEqual(beta.snapshotPartitions.map((partition) => partition.scope), ['Symbol=BETA'])
  assert.deepEqual(beta.bid.limits.map((limit) => limit.price), ['41.25'])
  assert.deepEqual(beta.ask.limits.map((limit) => limit.price), ['41.75'])
})

test('the global walk: one GLOBAL book per instant, pinned', () => {
  const walked = books(0, true)
  assert.ok(walked.every((book) => book.crosscode === graph.GLOBAL_SYMBOL))
  assert.deepEqual(hashes(walked), [
    '13912303688916928263',
    '15590660138982923340',
    '1351903172767778939',
    '7248446536207417329',
    '14863578545659580262',
    '2758054018770817390',
    '15235975123582398460',
    '8238562214873767668',
    '6680080629438284768',
  ])
})

test('a two-millisecond grid adds the ticks the walk emits, and changes no other book', () => {
  const walked = books(2)
  assert.equal(walked.length, 14)
  const ticks = walked.filter(
    (book) =>
      book.snapunix !== null && !book.bid.deltas.length && !book.ask.deltas.length && !book.executions.length,
  )
  assert.deepEqual(
    ticks.map((book) => [book.crosscode, book.snapunix - T0]),
    [
      ['ALPHA', 2_000_000n],
      ['ALPHA', 4_000_000n],
      ['ALPHA', 6_000_000n],
    ],
  )
  const plain = new Set(hashes(books()))
  assert.deepEqual(hashes(walked).filter((hash) => !plain.has(hash)), [
    '17020186527711404026',
    '14678968861820738602',
    '18061724431166054852',
  ])
})
