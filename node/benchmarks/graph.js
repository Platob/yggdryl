'use strict'

// Boundary cost of the graph vocabulary, against the native numbers.
//
// Every case here is one crossing over the typed market leaves and
// `MarketData`: building an order event from named facts, reading a fact back
// typed, dating and undating an element, a book folding a group of
// operations, the lazy book and event walks, and the lifted Arrow doors. Run
// against the release addon with `npm run --prefix node bench:graph`.

const { performance } = require('node:perf_hooks')

const { graph } = require('yggdryl')

const iterations = Number.parseInt(process.env.YGGDRYL_BENCH_ITERATIONS ?? '5000', 10)
if (!Number.isSafeInteger(iterations) || iterations <= 0) {
  throw new RangeError('YGGDRYL_BENCH_ITERATIONS must be a positive safe integer')
}

function benchmark(name, operation) {
  for (let index = 0; index < Math.min(iterations, 1_000); index += 1) operation()
  const started = performance.now()
  for (let index = 0; index < iterations; index += 1) operation()
  const elapsed = performance.now() - started
  const rate = Math.round((iterations * 1_000) / elapsed)
  console.log(`${name}: ${rate.toLocaleString('en-US')} operations/second`)
}

// A fold or a whole stream costs milliseconds, so it runs a fiftieth of the
// hit count.
function benchmarkStreams(name, operation) {
  const rounds = Math.max(1, Math.round(iterations / 50))
  for (let index = 0; index < Math.min(rounds, 5); index += 1) operation()
  const started = performance.now()
  for (let index = 0; index < rounds; index += 1) operation()
  const elapsed = performance.now() - started
  const rate = Math.round((rounds * 1_000) / elapsed)
  console.log(`${name}: ${rate.toLocaleString('en-US')} operations/second`)
}

const FOLD_OPERATION_COUNT = 512
const CLOCK = 1_700_000_000_000_000_000n

function orderEvent(facts = {}) {
  return new graph.OrderEvent(CLOCK, {
    crosscode: 'G-1',
    side: 'BUY',
    ticker: 'ACME',
    price: '100.25',
    currency: 'USD',
    quantity: 10,
    ...facts,
  })
}

const ORDER_EVENT = orderEvent()
const ORDER = ORDER_EVENT.intoElement()
const DATA = new graph.MarketData(ORDER_EVENT)
const LANE = new graph.Lane({ price: '100.25', currency: 'USD', quantity: '10' })

// One order per price tick, all on the bid side of one symbol at one
// instant - the atomic group a book folds when it replays a session's orders.
const FOLD_OPERATIONS = Array.from({ length: FOLD_OPERATION_COUNT }, (_, index) =>
  orderEvent({ crosscode: `G-${index}`, price: String(100 + index) }))
const FOLD_BOOK = new graph.BookEvent(CLOCK, 'ACME').withOperations(FOLD_OPERATIONS)

function count(iterable) {
  let seen = 0
  for (const _ of iterable) seen += 1
  return seen
}

benchmark('order event from an object', () => orderEvent())
benchmark('order from an object', () => new graph.Order({ crosscode: 'G-1', side: 'BUY', price: '100.25' }))
benchmark('order event read price', () => ORDER_EVENT.price)
benchmark('order event read bid lane', () => ORDER_EVENT.bid)
benchmark('order event read altids map', () => ORDER_EVENT.altids)
benchmark('order at', () => ORDER.at(CLOCK))
benchmark('order event into element', () => ORDER_EVENT.intoElement())
benchmark('market data wrap', () => new graph.MarketData(ORDER_EVENT))
benchmark('market data into leaf', () => DATA.intoLeaf())
benchmark('order event toJSON', () => ORDER_EVENT.toJSON())
benchmark('lane from an object', () => new graph.Lane({ price: '100.25', currency: 'USD', quantity: '10' }))
benchmark('lane read price', () => LANE.price)
benchmarkStreams(`book fold/${FOLD_OPERATION_COUNT}`, () =>
  new graph.BookEvent(CLOCK, 'ACME').withOperations(FOLD_OPERATIONS))
benchmarkStreams(`book iterator drain/${FOLD_OPERATION_COUNT}`, () =>
  count(new graph.BookIterator(FOLD_OPERATIONS)))
benchmarkStreams(`event iterator drain/${FOLD_OPERATION_COUNT}`, () =>
  count(new graph.EventIterator(FOLD_OPERATIONS)))
benchmarkStreams(`operations arrowReader/${FOLD_OPERATION_COUNT}`, () =>
  graph.MarketData.arrowReader(FOLD_OPERATIONS).intoIpc())
benchmarkStreams(`operations fromArrowReader/${FOLD_OPERATION_COUNT}`, () =>
  count(graph.MarketData.fromArrowReader(graph.MarketData.arrowReader(FOLD_OPERATIONS))))
benchmarkStreams('book arrowReader', () => graph.MarketData.arrowReader([FOLD_BOOK]).intoIpc())
benchmarkStreams('book fromArrowReader', () =>
  count(graph.MarketData.fromArrowReader(graph.MarketData.arrowReader([FOLD_BOOK]))))
