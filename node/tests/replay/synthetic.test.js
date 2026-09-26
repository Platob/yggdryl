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

  const [expired] = leaves.filter((leaf) => leaf.state === '95EXPIRED')
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
  const walked = books()
  assert.deepEqual(hashes(walked), [
    '16242326805217559900',
    '16544763427349240156',
    '5235232494032010052',
    '1828812868823296846',
    '5034916044474474282',
    '7481385549412507034',
    '17553807238354852533',
    '3831896887608114243',
    '10840428696151966888',
    '3304533786340479661',
    '9496428554108646693',
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
    '2570637125084932438',
    '6528218759551834422',
    '5835780611225251832',
    '13146055324015594326',
    '10162744597707995629',
    '14867780961587921349',
    '12002803448621661912',
    '8948237090346574842',
    '3002296415775241335',
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
    '5253697755473297734',
    '9021731498757604473',
    '10641605480225750750',
  ])
})
