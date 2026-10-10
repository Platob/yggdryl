'use strict'

// Boundary cost of the graph vocabulary, against the native numbers.
//
// Every case here is one crossing over the typed market leaves and
// `MarketData`: building an order event from named facts, reading a fact back
// typed, dating and undating an element, a book folding a group of
// operations, its limits, one side's live entries and two-sided readings,
// the lazy book and event walks - one under a filter - the lifted Arrow
// doors and the named views over them, the identifier maps crossing as a
// plain object and the instruments' reads. Run against the release
// addon with `npm run --prefix node bench:graph`.

const { performance } = require('node:perf_hooks')

const { Identifiers, Instruments, graph } = require('yggdryl')

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
    side: 'BUYS',
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
benchmark('order from an object', () => new graph.Order({ crosscode: 'G-1', side: 'BUYS', price: '100.25' }))
benchmark('order event read price', () => ORDER_EVENT.price)
benchmark('order event read isincode', () => ORDER_EVENT.isincode)
benchmark('order event read fxrates', () => ORDER_EVENT.fxrates)
benchmark('order event read marketdatakind', () => ORDER_EVENT.marketdatakind)
benchmark('order event read identifiers set', () => ORDER_EVENT.identifiers)
benchmark('order at', () => ORDER.at(CLOCK))
benchmark('order event into element', () => ORDER_EVENT.intoElement())
benchmark('market data wrap', () => new graph.MarketData(ORDER_EVENT))
benchmark('market data into leaf', () => DATA.intoLeaf())
benchmark('order event toJSON', () => ORDER_EVENT.toJSON())
benchmarkStreams(`book fold/${FOLD_OPERATION_COUNT}`, () =>
  new graph.BookEvent(CLOCK, 'ACME').withOperations(FOLD_OPERATIONS))
benchmarkStreams(`book iterator drain/${FOLD_OPERATION_COUNT}`, () =>
  count(new graph.BookIterator(FOLD_OPERATIONS)))
// The same walk under a filter over the `marketdata` row, bound once when the
// walk opens and answered a batch of booked inputs at a time.
benchmarkStreams(`book iterator filtered drain/${FOLD_OPERATION_COUNT}`, () =>
  count(new graph.BookIterator(FOLD_OPERATIONS, 0, "side = 'BUYS'")))
benchmark('book keyed', () => graph.BookEvent.keyed(CLOCK, 'ACME'))
benchmarkStreams(`book bid alive/${FOLD_OPERATION_COUNT}`, () => FOLD_BOOK.aliveOn('BUYS'))
benchmarkStreams(`event iterator drain/${FOLD_OPERATION_COUNT}`, () =>
  count(new graph.EventIterator(FOLD_OPERATIONS)))
benchmarkStreams(`operations arrowReader/${FOLD_OPERATION_COUNT}`, () =>
  graph.MarketData.arrowReader(FOLD_OPERATIONS).intoIpc())
benchmarkStreams(`operations fromArrowReader/${FOLD_OPERATION_COUNT}`, () =>
  count(graph.MarketData.fromArrowReader(graph.MarketData.arrowReader(FOLD_OPERATIONS))))
benchmarkStreams('book arrowReader', () => graph.MarketData.arrowReader([FOLD_BOOK]).intoIpc())
benchmarkStreams('book fromArrowReader', () =>
  count(graph.MarketData.fromArrowReader(graph.MarketData.arrowReader([FOLD_BOOK]))))
// The book readings over the folded bid side - one limit per tick - and the
// named views: a plan built from its spelling, and the orders view over the
// operations' own stream.
benchmarkStreams(`book bid limits/${FOLD_OPERATION_COUNT}`, () => FOLD_BOOK.limits('BUYS'))
benchmark('book bid depth/10', () => FOLD_BOOK.depth('BUYS', 10))
benchmarkStreams(`book alive/${FOLD_OPERATION_COUNT}`, () => FOLD_BOOK.alive())
benchmarkStreams(`book ordlive/${FOLD_OPERATION_COUNT}`, () => FOLD_BOOK.ordlive())
benchmarkStreams(`book orddelta/${FOLD_OPERATION_COUNT}`, () => FOLD_BOOK.orddelta())
benchmark('book spread', () => FOLD_BOOK.spread)
benchmark('book imbalance/10', () => FOLD_BOOK.imbalance(10))
const IDENTIFIERS_OBJECT = Object.fromEntries([
  ...Array.from({ length: 8 }, (_, index) => [`oms:k${index}`, `V-${index}`]),
  ['isin', 'US0378331005'],
  ['ullink:isin', 'US0378331005'],
])
const IDENTIFIERS = Identifiers.fromObject(IDENTIFIERS_OBJECT)
const ZERO = '00000000-0000-0000-0000-000000000000'
const INSTRUMENTS = new Instruments()
INSTRUMENTS.merge({
  uuid: ZERO,
  crossuuid: ZERO,
  crosscode: '',
  hashcode: 0n,
  crosshashcode: 0n,
  placeholder: false,
  isin: 'CH0012214059',
  cficode: 'ESVUFR',
  listings: [{
    miccode: 'XSWX',
    ticker: 'HOLN',
    currency: null,
    codes: new Map([['bloomberg', 'HOLN SW Equity'], ['ric', 'HOLN.S']]),
  }],
})
benchmark(`identifiers fromObject/${IDENTIFIERS.length}`, () => Identifiers.fromObject(IDENTIFIERS_OBJECT))
benchmark(`identifiers intoObject/${IDENTIFIERS.length}`, () => IDENTIFIERS.intoObject())
benchmark('instruments get', () => INSTRUMENTS.get('CH0012214059'))
benchmark('instruments getByTicker', () => INSTRUMENTS.getByTicker('HOLN', 'XSWX'))
benchmark('market view plan', () => graph.MarketData.plan('orders', ["securityids['isin'] as isin"]))
benchmarkStreams(`market view orders/${FOLD_OPERATION_COUNT}`, () =>
  graph.MarketData.applyView('orders', graph.MarketData.arrowReader(FOLD_OPERATIONS)).intoIpc())
