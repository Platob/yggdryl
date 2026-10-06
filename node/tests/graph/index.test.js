'use strict'

// `graph`: the frozen namespace `node/src/graph/mod.rs` binds - every class
// grouped the way `fix` and `iceberg` are, the module's own constants, the
// enum listings, and the named-fact door every operation leaf shares. What
// each class does is pinned in its own file beside this one.

const assert = require('node:assert/strict')
const test = require('node:test')

const yggdryl = require('yggdryl')
const { Scalar, enums, graph } = yggdryl

const CLASSES = [
  'BookRef',
  'Order',
  'Quote',
  'Execution',
  'OrderEvent',
  'QuoteEvent',
  'ExecutionEvent',
  'TradeEvent',
  'BookEvent',
  'SnapshotEvent',
  'MarketData',
  'MarketDataRowIterator',
  'BookIterator',
  'EventIterator',
  'Candle',
  'CandleOptions',
  'CandleIterator',
]

test('every native class is reached through the namespace, and only there', () => {
  assert.deepEqual(
    Object.keys(graph).sort(),
    [...CLASSES, 'candles', 'ENTRY_ID', 'ENTRY_REF_ID'].sort(),
  )
  for (const name of CLASSES) {
    assert.equal(typeof graph[name], 'function', name)
    assert.equal(yggdryl[name], undefined, name)
    assert.equal(yggdryl[`Js${name}`], undefined, name)
  }
  // The one function beside the classes: the candle walk, drained.
  assert.equal(typeof graph.candles, 'function')
  assert.equal(yggdryl.candles, undefined)
})

test('no retired name survives', () => {
  for (const name of [
    'MarketEnvelope',
    'MarketOperation',
    'Trade',
    'Book',
    'BookControl',
    'BookInput',
    'BookRowIterator',
    'BookInputRowIterator',
    // A1/A2/A10: the book is its own readings; the lanes and the side
    // leaves are gone, and so is the one consolidated book.
    'Lane',
    'BookSide',
    'SnapshotPartition',
    'GLOBAL_SYMBOL',
  ]) {
    assert.equal(graph[name], undefined, name)
    assert.equal(yggdryl[name], undefined, name)
  }
  assert.equal(enums.operationKinds, undefined)
  assert.equal(graph.MarketData._arrowReaderNative, undefined)
  assert.equal(yggdryl._marketDataArrowReaderNative, undefined)
  assert.equal(graph.BookIterator._bookIteratorNative, undefined)
  assert.equal(graph.EventIterator._eventIteratorNative, undefined)
  assert.equal(graph.CandleIterator._candleIteratorNative, undefined)
  assert.equal(graph.Candle._fromScalarNative, undefined)
  assert.equal(yggdryl._graphGlobalSymbolNative, undefined)
})

test('the namespace is frozen', () => {
  assert.ok(Object.isFrozen(graph))
  assert.throws(() => { graph.Order = null }, TypeError)
})

test('the two constants are exported, and no followed list: a leaf follows every identifier it lacks', () => {
  assert.equal(graph.ENTRY_ID, 'mdentryid')
  assert.equal(graph.ENTRY_REF_ID, 'mdentryrefid')
  assert.equal('FOLLOWED_IDENTIFIERS' in graph, false)
})

test('the enum listings name the column vocabulary and the market kinds', () => {
  assert.deepEqual(enums.marketKinds, graph.MarketData.kinds())
  assert.equal(enums.marketKinds.length, 10)
  assert.equal(enums.marketKinds.at(-1), 'fix')
  assert.ok(Object.isFrozen(enums.marketKinds))
  assert.ok(enums.mdUpdateActions.includes('snapshot'))
  assert.equal(enums.elementColumns.length, 6)
  assert.equal(enums.eventColumns.length, 9)
  assert.equal(enums.marketColumns.length, 35)
  assert.equal(enums.operationColumns.length, 5)
  assert.deepEqual(enums.operationColumns, ['ordqty', 'timeinforce', 'tradable', 'identifiers', 'partyids'])
  // When an element last executed is a market fact, never an event's.
  assert.ok(enums.marketColumns.includes('execunix'))
  assert.equal(enums.eventColumns.includes('execunix'), false)
  for (const column of ['isincode', 'fxrates', 'bidpx', 'bidqty', 'bidccy', 'askpx', 'askqty', 'askccy']) {
    assert.ok(enums.marketColumns.includes(column), column)
  }
  // A named fact is a column name: every one the three listings spell is a
  // getter of an operation event.
  const event = new graph.OrderEvent(1, { crosscode: 'O-1' })
  for (const column of [...enums.elementColumns, ...enums.eventColumns, ...enums.marketColumns, ...enums.operationColumns]) {
    assert.ok(column in event, column)
  }
})

test('a fact given as undefined is skipped and null clears', () => {
  const plain = new graph.OrderEvent(1, { crosscode: 'X' })
  assert.ok(new graph.OrderEvent(1, { crosscode: 'X', ticker: undefined }).equals(plain))
  assert.ok(new graph.OrderEvent(1, { crosscode: 'X', book: undefined }).equals(plain))
  assert.ok(new graph.OrderEvent(1, { crosscode: 'X', book: null }).equals(plain))
  const stated = {
    crosscode: 'X', price: '1', ticker: 'T', timeinforce: '0', identifiers: [new yggdryl.Identifier('orderid', 'X')],
  }
  const cleared = new graph.OrderEvent(1, { ...stated, ticker: null, price: null, timeinforce: null, identifiers: null })
  assert.equal(cleared.ticker, null)
  assert.equal(cleared.price, null)
  assert.equal(cleared.timeinforce, null)
  assert.equal(cleared.identifiers.length, 0)
  assert.ok(cleared.equals(plain))
  assert.ok(!cleared.equals(new graph.OrderEvent(1, stated)))
})

test('a fact is resolved folded, checked by its column and refused by name', () => {
  assert.equal(new graph.OrderEvent(1, { PRICE: '1' }).price, new graph.OrderEvent(1, { price: '1' }).price)
  assert.throws(() => new graph.OrderEvent(1, { bogus: 1 }), /OrderEvent states no fact "bogus"/)
  assert.throws(() => new graph.Quote({ bogus: 1 }), /Quote states no fact "bogus"/)
  assert.throws(() => new graph.OrderEvent(1, { price: 'not a number' }), /\$\.price/)
  assert.throws(() => new graph.Order({ side: 'SIDEWAYS' }), /side/)
  assert.throws(() => new graph.Order({ currunix: 1 }), /an undated element has no clock, state or chain/)
  assert.throws(() => new graph.Order({ book: 1 }), /Order states no fact "book"/)
  assert.throws(() => new graph.OrderEvent(1, [1]), /facts must be an object keyed by column name/)
  assert.throws(() => graph.OrderEvent(1), /cannot be invoked without 'new'/)
})

test('the market facts cross as plain values', () => {
  const event = new graph.OrderEvent(1, {
    crosscode: 'O-1',
    side: 'BUYS',
    securityids: [new yggdryl.Identifier('isin', 'US0378331005')],
    bidpx: '100.5',
    bidqty: 3,
    bidccy: 'EUR',
  })
  assert.equal(event.isincode, 'US0378331005')
  assert.equal(event.bidpx, '100.5')
  assert.equal(event.bidqty, '3')
  assert.equal(event.bidccy, 'EUR')
  assert.equal(event.askpx, null)
  assert.equal(event.askccy, null)
  // Nothing fills the rates, so an element states none unless given.
  assert.deepEqual(event.fxrates, {})
  assert.equal(event.marketdatakind, 'ORDR')
  // The stored cross code is the kind, the side, then the base.
  assert.equal(event.crosscode, '10:1:O-1')
  assert.equal(new graph.OrderEvent(1, { crosscode: 'O-1', side: 'SELL' }).crosscode, '10:2:O-1')
  assert.equal(new graph.OrderEvent(1, { crosscode: 'O-1' }).side, 'UNKN')
  assert.equal(new graph.OrderEvent(1, { crosscode: 'O-1' }).crosscode, '10:0:O-1', 'a side nobody stated is 0')
  assert.equal(new graph.OrderEvent(1, { crosscode: 'O-1' }).isincode, null)
})

test('every element states its cross code as {kind}:{side}:{base}', () => {
  const stored = (Class, facts) => new Class(1, facts).crosscode
  // The kind is the `MarketDataKind` code and the side the `Side` code of a
  // sided kind - an order, an execution - and 0 for any other kind: a quote
  // holds its two legs, its side a tag that moves no prefix.
  assert.equal(stored(graph.OrderEvent, { crosscode: 'ORD-1', side: 'BUYS' }), '10:1:ORD-1')
  assert.equal(stored(graph.OrderEvent, { crosscode: 'ORD-1', side: 'SELL' }), '10:2:ORD-1')
  assert.equal(stored(graph.OrderEvent, { crosscode: 'ORD-1' }), '10:0:ORD-1', 'a side nobody stated is 0')
  assert.equal(stored(graph.QuoteEvent, { crosscode: 'Q-1', side: 'BUYS' }), '14:0:Q-1')
  assert.equal(stored(graph.ExecutionEvent, { crosscode: 'E-1', side: 'SELL' }), '8:2:E-1')
})

test('a stored cross code replaces another kind or side prefix and an empty code stays empty', () => {
  const order = (facts) => new graph.OrderEvent(1, facts).crosscode
  assert.equal(order({ crosscode: '8:2:ORD-1', side: 'BUYS' }), '10:1:ORD-1', 'the prefix is replaced')
  assert.equal(order({ crosscode: '10:1:ORD-1', side: 'BUYS' }), '10:1:ORD-1', 'a stored code is itself')
  assert.equal(order({ crosscode: '', side: 'BUYS' }), '', 'no code, no prefix')
  // A book is never codeless: an empty symbol keys it by the ISIN that
  // states none, and it states no ticker.
  assert.equal(new graph.BookEvent(1, '').crosscode, '3:0:XX0000000000')
  assert.equal(new graph.BookEvent(1, '').ticker, null)
})

test('a book states 3:0:{ticker} and every identity derives from the stored code', () => {
  assert.equal(new graph.BookEvent(1, 'AAPL').crosscode, '3:0:AAPL')
  assert.equal(new graph.BookEvent(1, 'XNAS:ESVUFR').crosscode, '3:0:XNAS:ESVUFR')
  // The cross hash is the XXH3-64 of the stored, prefixed code, and the cross
  // identity follows it: the same base on the other side is another chain.
  const buy = new graph.OrderEvent(1, { crosscode: 'ORD-1', side: 'BUYS' })
  const sell = new graph.OrderEvent(1, { crosscode: 'ORD-1', side: 'SELL' })
  assert.equal(buy.crosshashcode, yggdryl.xxhash.xxh3(Buffer.from('10:1:ORD-1')))
  assert.equal(sell.crosshashcode, yggdryl.xxhash.xxh3(Buffer.from('10:2:ORD-1')))
  assert.notEqual(buy.crossuuid, sell.crossuuid)
  assert.notEqual(buy.crosshashcode, sell.crosshashcode)
  assert.equal(buy.crossuuid, new graph.OrderEvent(2, { crosscode: 'ORD-1', side: 'BUYS' }).crossuuid, 'one chain')
})

test('a fact record crosses as a Scalar too', () => {
  const record = Scalar.from({ crosscode: 'X', price: '2' })
  assert.ok(new graph.OrderEvent(1, record).equals(new graph.OrderEvent(1, { crosscode: 'X', price: '2' })))
})
