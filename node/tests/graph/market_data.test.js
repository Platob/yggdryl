'use strict'

// `graph.MarketData` and its lifted Arrow doors: `node/src/graph/market_data.rs`.

const assert = require('node:assert/strict')
const test = require('node:test')

const arrow = require('apache-arrow')

const { BatchReader, Field, graph } = require('yggdryl')

const CLOCK = 1_700_000_000_000_000_000n

// One leaf of every kind, in `MarketData.kinds()` order.
function leaves() {
  const order = new graph.OrderEvent(CLOCK, { crosscode: 'O-1', side: 'BUY', price: '101', quantity: 5, ticker: 'ACME' })
  const quote = new graph.QuoteEvent(CLOCK, { crosscode: 'Q-1', side: 'SELL', price: '102', quantity: 3, ticker: 'ACME' })
  const execution = new graph.ExecutionEvent(CLOCK + 1n, { crosscode: 'O-1', side: 'BUY', lastpx: '101', lastqty: 5 })
  const trade = graph.TradeEvent.fromParts(new graph.ExecutionEvent(CLOCK, { crosscode: 'T-1', ticker: 'ACME' }), [
    new graph.ExecutionEvent(CLOCK, { crosscode: 'E-1', side: 'BUY', lastpx: '1', lastqty: 1 }),
    new graph.ExecutionEvent(CLOCK, { crosscode: 'E-2', side: 'SELL', lastpx: '1', lastqty: 1 }),
  ])
  const book = new graph.BookEvent(CLOCK, 'ACME').withOperations([order, quote])
  return [
    new graph.Order({ crosscode: 'O-1', price: '101' }),
    new graph.Quote({ crosscode: 'Q-1' }),
    new graph.Execution({ crosscode: 'E-1', lastqty: 2 }),
    new graph.BookSide('BUY').withOperation(order),
    order,
    quote,
    execution,
    trade,
    book,
    graph.SnapshotEvent.snapshot(order, 'S'),
  ]
}

// Every value a reader yields, drained.
function drain(rows) {
  return [...rows]
}

test('every leaf wraps and names its kind', () => {
  const items = leaves()
  const wrapped = items.map((leaf) => new graph.MarketData(leaf))
  assert.deepEqual(wrapped.map((data) => data.kind), graph.MarketData.kinds())
  assert.deepEqual(wrapped.map((data) => data.isEvent), [...Array(4).fill(false), ...Array(6).fill(true)])
  items.forEach((leaf, at) => {
    const data = wrapped[at]
    assert.ok(data.intoLeaf() instanceof leaf.constructor)
    assert.ok(data.intoLeaf().equals(leaf))
    assert.ok(new graph.MarketData(data).equals(data))
    // The element and market facts delegate to the leaf.
    for (const name of ['curruuid', 'crosscode', 'price', 'side', 'currhashcode']) {
      assert.equal(data[name], leaf[name], name)
    }
  })
})

test('asLeaf borrows the leaf it is and null otherwise', () => {
  const order = leaves()[4]
  const data = new graph.MarketData(order)
  assert.ok(data.asOrderEvent().equals(order))
  assert.ok(data.intoLeaf() instanceof graph.OrderEvent)
  for (const name of ['asOrder', 'asQuote', 'asExecution', 'asBookSide', 'asQuoteEvent',
    'asExecutionEvent', 'asTradeEvent', 'asBookEvent', 'asSnapshotEvent']) {
    assert.equal(data[name](), null, name)
  }
})

test('book answers the control of an operation or a snapshot', () => {
  const control = new graph.BookRef({ action: '0', scope: 'S' })
  assert.ok(new graph.MarketData(new graph.QuoteEvent(CLOCK, { book: control })).book.equals(control))
  assert.equal(new graph.MarketData(leaves()[9]).book.action, 'snapshot')
  assert.equal(new graph.MarketData(new graph.Order()).book, null)
})

test('anything but a leaf is refused', () => {
  assert.throws(() => new graph.MarketData(1), /expected MarketData or a market leaf, got number/)
  assert.throws(() => new graph.MarketData(null), /expected MarketData or a market leaf, got null/)
})

test('following crosses operation kinds and merging no variant', () => {
  const first = new graph.MarketData(new graph.OrderEvent(CLOCK, { crosscode: 'O-1', price: '1' }))
  const later = new graph.MarketData(new graph.OrderEvent(CLOCK + 1n, { crosscode: 'O-1', price: '2' }))
  const followed = later.withPrevious(first)
  assert.equal(followed.kind, 'order_event')
  assert.equal(followed.asOrderEvent().prevuuid, first.curruuid)
  // An execution follows the order it fills across kinds, keeping its own.
  const execution = new graph.MarketData(new graph.ExecutionEvent(CLOCK + 1_000_000n, { crosscode: 'O-1' }))
  const fill = execution.withPrevious(first)
  assert.equal(fill.kind, 'execution_event')
  assert.equal(fill.asExecutionEvent().prevuuid, first.curruuid)
  // A book side follows no other variant, and a merge never crosses one.
  assert.equal(new graph.MarketData(new graph.BookSide('BUY')).withPrevious(first), null)
  assert.equal(first.mergeWith(execution), null)
  assert.equal(first.mergeWith(first), null)
  assert.ok(later.isAfter(first) && first.isBefore(later))
  // An undated value is neither after nor before anything.
  assert.equal(new graph.MarketData(new graph.Order()).isAfter(first), false)
})

test('equals, stableHash, toString, clone and toJSON round trip every kind', () => {
  for (const leaf of leaves()) {
    const data = new graph.MarketData(leaf)
    const twin = graph.MarketData.fromJSON(data.toJSON())
    assert.ok(twin.equals(data), data.kind)
    assert.equal(twin.stableHash(), data.stableHash())
    assert.equal(data.stableHash(), leaf.stableHash())
    assert.ok(data.clone().equals(data))
    assert.equal(
      data.toString(),
      `MarketData(${data.curruuid}, kind="${data.kind}", crosscode="${data.crosscode}")`,
    )
    // A leaf and its `MarketData` write the same one-row stream.
    assert.equal(leaf.toJSON(), data.toJSON())
    assert.ok(leaf.constructor.fromJSON(leaf.toJSON()).equals(leaf))
  }
})

test('the field is the lifted marketdata struct', () => {
  const field = graph.MarketData.field()
  assert.ok(field instanceof Field)
  assert.equal(field.name, 'marketdata')
  assert.equal(field.nullable, false)
  const names = Array.from({ length: field.fieldLen }, (_, at) => field.fieldAt(at).name)
  assert.equal(names[0], 'kind')
  for (const name of ['currunix', 'price', 'altids', 'bid', 'ask', 'mdupdateaction', 'executions',
    'bidside', 'askside', 'snapshotpartitions', 'live', 'deltas']) {
    assert.ok(names.includes(name), name)
  }
})

test('arrowReader then fromArrowReader round trips every variant', () => {
  const items = leaves()
  const reader = graph.MarketData.arrowReader(items)
  assert.ok(reader instanceof BatchReader)
  assert.ok(reader.field.equals(graph.MarketData.field()))
  const back = drain(graph.MarketData.fromArrowReader(reader))
  assert.deepEqual(back.map((data) => data.kind), graph.MarketData.kinds())
  back.forEach((data, at) => {
    assert.ok(data instanceof graph.MarketData)
    assert.ok(data.intoLeaf().equals(items[at]), data.kind)
  })
})

test('the stream crosses a BatchReader round trip through IPC and Arrow JS', () => {
  const items = leaves()
  const bytes = graph.MarketData.arrowReader(items).intoIpc()
  const viaIpc = drain(graph.MarketData.fromArrowReader(BatchReader.fromIpc(bytes)))
  assert.equal(viaIpc.length, 10)
  viaIpc.forEach((data, at) => assert.ok(data.intoLeaf().equals(items[at]), data.kind))
  const table = graph.MarketData.arrowReader(items).intoTable()
  assert.equal(table.numRows, 10)
  assert.deepEqual([...table.getChild('kind')], graph.MarketData.kinds())
  const viaTable = drain(graph.MarketData.fromArrowReader(BatchReader.from(table)))
  viaTable.forEach((data, at) => assert.ok(data.intoLeaf().equals(items[at]), data.kind))
})

test('arrowReader takes MarketData and bounds its batches', () => {
  const items = [0, 1, 2, 3, 4].map((step) =>
    new graph.MarketData(new graph.OrderEvent(CLOCK + BigInt(step), { crosscode: `O-${step}` })))
  const table = graph.MarketData.arrowReader(items.values(), 2).intoTable()
  assert.deepEqual(table.batches.map((batch) => batch.numRows), [2, 2, 1])
  assert.equal(graph.MarketData.arrowReader([]).intoTable().numRows, 0)
})

test('arrowReader pulls lazily and surfaces a JavaScript failure', () => {
  const pulled = []
  function* items() {
    for (let step = 0; step < 3; step += 1) {
      pulled.push(step)
      yield new graph.OrderEvent(CLOCK + BigInt(step))
    }
    throw new RangeError('the source gave up')
  }
  const reader = graph.MarketData.arrowReader(items())
  assert.deepEqual(pulled, [])
  assert.throws(() => reader.intoTable(), /the source gave up/)
  assert.throws(
    () => graph.MarketData.arrowReader([1]).intoTable(),
    /expected MarketData or a market leaf, got number/,
  )
  assert.throws(() => graph.MarketData.arrowReader(5), /items must be an iterable/)
})

test('a lifecycle-shaped batch reads into events', () => {
  // Event and operation columns in their own order, a foreign column, no
  // book column, the names folded, the types castable: the door resolves
  // what it knows once and ignores the rest.
  const altids = new arrow.Map_(new arrow.Field('entries', new arrow.Struct([
    new arrow.Field('key', new arrow.Utf8(), false),
    new arrow.Field('value', new arrow.Utf8(), true),
  ]), false))
  const table = new arrow.Table({
    foreign: arrow.vectorFromArray([1, 2], new arrow.Int32()),
    CrossCode: arrow.vectorFromArray(['O-1', 'O-1'], new arrow.Utf8()),
    Kind: arrow.vectorFromArray(['order_event', 'execution_event'], new arrow.Utf8()),
    currunix: arrow.vectorFromArray([CLOCK, CLOCK + 1n], new arrow.Int64()),
    side: arrow.vectorFromArray(['BUY', 'BUY'], new arrow.Utf8()),
    price: arrow.vectorFromArray(['101', null], new arrow.Utf8()),
    lastqty: arrow.vectorFromArray([null, '5'], new arrow.Utf8()),
    altids: arrow.vectorFromArray([new Map([['ORDERID', 'O-1']]), null], altids),
  })
  const [order, execution] = drain(graph.MarketData.fromArrowReader(BatchReader.from(table)))
    .map((data) => data.intoLeaf())
  assert.ok(order instanceof graph.OrderEvent)
  assert.ok(execution instanceof graph.ExecutionEvent)
  assert.equal(order.currunix, CLOCK)
  assert.equal(order.crosscode, 'O-1')
  assert.equal(order.side, 'BUY')
  assert.equal(order.price, '101')
  assert.deepEqual(order.altids, { ORDERID: 'O-1' })
  assert.equal(execution.lastqty, '5')
  assert.equal(execution.currunix, CLOCK + 1n)
})

test('fromArrowReader refuses by name and fuses', () => {
  const read = (columns) => drain(graph.MarketData.fromArrowReader(BatchReader.from(new arrow.Table(columns))))
  assert.throws(
    () => read({ currunix: arrow.vectorFromArray([CLOCK], new arrow.Int64()) }),
    /\$\[0\]\.kind: expected order, quote.*got null/,
  )
  assert.throws(() => read({ kind: arrow.vectorFromArray(['nope'], new arrow.Utf8()) }), /\$\[0\]\.kind: .*got "nope"/)
  const rows = graph.MarketData.fromArrowReader(BatchReader.from(new arrow.Table({
    kind: arrow.vectorFromArray(['order_event', 'order_event'], new arrow.Utf8()),
    currunix: arrow.vectorFromArray([CLOCK, null], new arrow.Int64()),
  })))
  assert.equal(rows.next().value.kind, 'order_event')
  assert.throws(() => rows.next(), /\$\[1\]\.currunix: expected the instant a dated leaf happened at/)
  assert.deepEqual(rows.next(), { value: undefined, done: true })
})

test('an undated row needs no clock', () => {
  const [data] = drain(graph.MarketData.fromArrowReader(BatchReader.from(new arrow.Table({
    kind: arrow.vectorFromArray(['order'], new arrow.Utf8()),
    crosscode: arrow.vectorFromArray(['X'], new arrow.Utf8()),
  }))))
  assert.equal(data.kind, 'order')
  assert.equal(data.crosscode, 'X')
  assert.ok(data.asOrder().equals(new graph.Order({ crosscode: 'X' })))
})
