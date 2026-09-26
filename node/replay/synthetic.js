'use strict'

// The synthetic replay: a deterministic market over two tickers, `ALPHA` and
// `BETA`, built from the native leaf constructors at fixed instants - no
// clock, no randomness - so every book the walk answers, and its
// `stableHash`, is the same on every machine. Per ticker: a ladder of three
// bid and three ask quotes, limit orders, executions stating `lastpx` and
// `lastqty`; `ALPHA` adds a market order stating no price and one composite
// trade over two sided fills, `BETA` an order that expires - its later
// statement `95EXPIRED`, following the first by `prevuuid` - and a full
// snapshot control that replaces its book. Two `ALPHA` statements stand one
// nanosecond apart, so a consumer can prove two such instants stay distinct.

const { graph } = require('../binding.js')

const { walk } = require('./walk.js')

/** The first instant of the scenario, nanoseconds since the epoch. */
const T0 = 1_700_000_000_000_000_000n
const MS = 1_000_000n

/** The two tickers the scenario trades. */
const SYMBOLS = Object.freeze(['ALPHA', 'BETA'])

// A quote belongs to its ticker's book scope, which is what a full snapshot
// of that scope replaces.
function quote(at, ticker, code, side, price, quantity) {
  const book = new graph.BookRef({ scope: `Symbol=${ticker}` })
  return new graph.QuoteEvent(at, { crosscode: code, ticker, side, price, quantity, currency: 'USD', book })
}

function order(at, ticker, code, side, price, quantity, facts = {}) {
  return new graph.OrderEvent(at, { crosscode: code, ticker, side, price, quantity, currency: 'USD', ...facts })
}

function fill(at, ticker, code, side, lastpx, lastqty) {
  return new graph.ExecutionEvent(at, { crosscode: code, ticker, side, lastpx, lastqty, currency: 'USD', state: 'FILLED' })
}

/** One ticker's ladder: three bid and three ask quotes at one instant. */
function ladder(at, ticker, bids, asks) {
  return [
    ...bids.map(([price, quantity], level) => quote(at, ticker, `${ticker}-QB${level + 1}`, 'BUY', price, quantity)),
    ...asks.map(([price, quantity], level) => quote(at, ticker, `${ticker}-QA${level + 1}`, 'SELL', price, quantity)),
  ]
}

/**
 * The scenario's operations, in the order the walk reads them: every leaf a
 * `MarketData`, nondecreasing by instant.
 */
function synthetic() {
  const expiring = order(T0 + 2n * MS, 'BETA', 'BETA-O-1', 'SELL', '41.5', 60, { state: 'NEW' })
  const expired = order(T0 + 4n * MS, 'BETA', 'BETA-O-1', 'SELL', '41.5', 60, { state: '95EXPIRED' }).withPrevious(expiring)
  const tradeAt = T0 + 5n * MS
  const trade = graph.TradeEvent.fromParts(
    new graph.ExecutionEvent(tradeAt, { crosscode: 'ALPHA-T-1', ticker: 'ALPHA', lastpx: '82.5', lastqty: 30, currency: 'USD' }),
    [
      fill(tradeAt, 'ALPHA', 'ALPHA-T-1-B', 'BUY', '82.5', 30),
      fill(tradeAt, 'ALPHA', 'ALPHA-T-1-S', 'SELL', '82.5', 30),
    ],
  )
  const snapshotAt = T0 + 6n * MS
  const leaves = [
    ...ladder(T0, 'ALPHA', [['82', 100], ['81.5', 200], ['81', 300]], [['82.5', 100], ['83', 150], ['83.5', 250]]),
    ...ladder(T0, 'BETA', [['41', 80], ['40.5', 120], ['40', 160]], [['41.5', 90], ['42', 110], ['42.5', 130]]),
    order(T0 + MS, 'ALPHA', 'ALPHA-O-1', 'BUY', '82', 50),
    // One nanosecond later: its own book, one instant apart.
    order(T0 + MS + 1n, 'ALPHA', 'ALPHA-O-2', 'SELL', '83', 25),
    expiring,
    // A market order: it states no price and rests at the unpriced limit.
    new graph.OrderEvent(T0 + 3n * MS, { crosscode: 'ALPHA-O-3', ticker: 'ALPHA', side: 'BUY', quantity: 40, currency: 'USD' }),
    fill(T0 + 3n * MS, 'BETA', 'BETA-E-1', 'SELL', '41.5', 10),
    expired,
    fill(T0 + 4n * MS + 500_000n, 'ALPHA', 'ALPHA-E-1', 'BUY', '82.5', 20),
    trade,
    // A full snapshot of BETA: the control, then the book it replaces with.
    graph.SnapshotEvent.snapshot(quote(snapshotAt, 'BETA', 'BETA-QB1', 'BUY', '41.25', 80), 'Symbol=BETA'),
    quote(snapshotAt, 'BETA', 'BETA-QB1', 'BUY', '41.25', 80),
    quote(snapshotAt, 'BETA', 'BETA-QA1', 'SELL', '41.75', 90),
  ]
  return leaves.map((leaf) => new graph.MarketData(leaf))
}

/** The books the native walk answers over the scenario. */
function books(snapshotMillis = 0, global = false) {
  return walk(synthetic(), { snapshotMillis, global })
}

synthetic.books = books
synthetic.T0 = T0
synthetic.SYMBOLS = SYMBOLS

module.exports = { synthetic, books, T0, SYMBOLS }
