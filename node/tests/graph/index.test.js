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
  'Lane',
  'BookRef',
  'Order',
  'Quote',
  'Execution',
  'OrderEvent',
  'QuoteEvent',
  'ExecutionEvent',
  'TradeEvent',
  'SnapshotPartition',
  'BookSide',
  'BookEvent',
  'SnapshotEvent',
  'MarketData',
  'MarketDataRowIterator',
  'BookIterator',
  'EventIterator',
]

test('every native class is reached through the namespace, and only there', () => {
  assert.deepEqual(
    Object.keys(graph).sort(),
    [...CLASSES, 'GLOBAL_SYMBOL', 'ENTRY_ID', 'ENTRY_REF_ID', 'FOLLOWED_ALTIDS'].sort(),
  )
  for (const name of CLASSES) {
    assert.equal(typeof graph[name], 'function', name)
    assert.equal(yggdryl[name], undefined, name)
    assert.equal(yggdryl[`Js${name}`], undefined, name)
  }
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
  ]) {
    assert.equal(graph[name], undefined, name)
    assert.equal(yggdryl[name], undefined, name)
  }
  assert.equal(enums.operationKinds, undefined)
  assert.equal(graph.MarketData._arrowReaderNative, undefined)
  assert.equal(yggdryl._marketDataArrowReaderNative, undefined)
  assert.equal(graph.BookIterator._bookIteratorNative, undefined)
  assert.equal(graph.EventIterator._eventIteratorNative, undefined)
})

test('the namespace is frozen', () => {
  assert.ok(Object.isFrozen(graph))
  assert.throws(() => { graph.Order = null }, TypeError)
})

test('the four constants are exported, the altids a frozen array', () => {
  assert.equal(graph.GLOBAL_SYMBOL, 'GLOBAL')
  assert.equal(graph.ENTRY_ID, 'MDENTRYID')
  assert.equal(graph.ENTRY_REF_ID, 'MDENTRYREFID')
  assert.ok(Array.isArray(graph.FOLLOWED_ALTIDS))
  assert.ok(Object.isFrozen(graph.FOLLOWED_ALTIDS))
  assert.ok(graph.FOLLOWED_ALTIDS.length > 0)
  assert.ok(graph.FOLLOWED_ALTIDS.every((name) => typeof name === 'string'))
})

test('the enum listings name the column vocabulary and the market kinds', () => {
  assert.deepEqual(enums.marketKinds, graph.MarketData.kinds())
  assert.equal(enums.marketKinds.length, 10)
  assert.ok(Object.isFrozen(enums.marketKinds))
  assert.ok(enums.mdUpdateActions.includes('snapshot'))
  assert.equal(enums.eventColumns.length, 16)
  assert.equal(enums.marketColumns.length, 19)
  assert.equal(enums.operationColumns.length, 8)
  // A named fact is a column name: every one the three listings spell is a
  // getter of an operation event.
  const event = new graph.OrderEvent(1, { crosscode: 'O-1' })
  for (const column of [...enums.eventColumns, ...enums.marketColumns, ...enums.operationColumns]) {
    assert.ok(column in event, column)
  }
})

test('a fact given as undefined is skipped and null clears', () => {
  const plain = new graph.OrderEvent(1, { crosscode: 'X' })
  assert.ok(new graph.OrderEvent(1, { crosscode: 'X', ticker: undefined }).equals(plain))
  assert.ok(new graph.OrderEvent(1, { crosscode: 'X', book: undefined }).equals(plain))
  assert.ok(new graph.OrderEvent(1, { crosscode: 'X', book: null }).equals(plain))
  const stated = { crosscode: 'X', price: '1', ticker: 'T', tif: '0', altids: { ORDERID: 'X' } }
  const cleared = new graph.OrderEvent(1, { ...stated, ticker: null, price: null, tif: null, altids: null })
  assert.equal(cleared.ticker, null)
  assert.equal(cleared.price, null)
  assert.equal(cleared.tif, null)
  assert.deepEqual(cleared.altids, {})
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

test('a fact record crosses as a Scalar too', () => {
  const record = Scalar.from({ crosscode: 'X', price: '2' })
  assert.ok(new graph.OrderEvent(1, record).equals(new graph.OrderEvent(1, { crosscode: 'X', price: '2' })))
})
