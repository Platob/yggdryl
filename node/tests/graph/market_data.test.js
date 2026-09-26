'use strict'

// `graph.MarketData` and its lifted Arrow doors: `node/src/graph/market_data.rs`.

const assert = require('node:assert/strict')
const test = require('node:test')

const arrow = require('apache-arrow')

const { BatchReader, Field, FieldPath, Plan, enums, graph } = require('yggdryl')

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
    'bidside', 'askside', 'snapshotpartitions', 'live', 'deltas', 'limits']) {
    assert.ok(names.includes(name), name)
  }
  // The three book facts follow the five book-control columns, and the
  // seven nested columns close the row.
  assert.equal(names.length, 59)
  assert.deepEqual(names.slice(49, 52), ['spread', 'crossed', 'locked'])
  assert.deepEqual(names.slice(52), NESTED)
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

// The named views over a `marketdata` stream: one plan each.
const NESTED = ['executions', 'bidside', 'askside', 'snapshotpartitions', 'live', 'deltas', 'limits']
const ISIN = 'US0378331005'

// The root's children, by name.
function rootNames() {
  const root = graph.MarketData.field()
  return Array.from({ length: root.fieldLen }, (_, at) => root.fieldAt(at).name)
}

// The children of one nested column's item, each under `prefix`.
function prefixed(nested, prefix) {
  const column = graph.MarketData.field().field(nested)
  const item = column.fieldLen === 1 && column.dtype.id.includes('serie') ? column.fieldAt(0) : column
  return Array.from({ length: item.fieldLen }, (_, at) => `${prefix}.${item.fieldAt(at).name}`)
}

// A stream of every leaf, with an order stating an ISIN and a chain of two.
function viewStream() {
  const identified = new graph.OrderEvent(CLOCK + 5n, {
    crosscode: 'O-5', side: 'BUY', price: '100', ticker: 'ACME', securityids: { ISIN },
  })
  const first = new graph.OrderEvent(CLOCK + 10n, { crosscode: 'C-1', side: 'BUY', price: '1' })
  const second = new graph.OrderEvent(CLOCK + 20n, { crosscode: 'C-1', side: 'BUY', price: '2' }).withPrevious(first)
  return graph.MarketData.arrowReader([second, ...leaves(), identified, first])
}

function viewed(view, lifts, crosscode) {
  return graph.MarketData.applyView(view, viewStream(), lifts, crosscode).intoTable()
}

function names(table) {
  return table.schema.fields.map((field) => field.name)
}

test('the market views are the enumeration the core lists', () => {
  assert.deepEqual([...enums.marketViews], [
    'orders', 'quotes', 'executions', 'trades', 'book_sides', 'books', 'lifecycle',
  ])
  assert.ok(Object.isFrozen(enums.marketViews))
})

test('every view is one plan whose text reads back', () => {
  const nested = NESTED.join(', ')
  const lift = "securityids['ISIN'] as isin"
  assert.equal(
    graph.MarketData.plan('orders', [lift]).toString(),
    `select * exclude (${nested}), ${lift} where kind in ('order', 'order_event')`,
  )
  assert.equal(
    graph.MarketData.plan('trades').toString(),
    `select * exclude (${nested}), unnest(executions) as execution where kind = 'trade_event'`,
  )
  assert.equal(
    graph.MarketData.plan('BOOK_SIDES').toString(),
    "select currunix, snapunix, unnest([bidside, askside]) as side where kind = 'book_event'",
  )
  assert.equal(
    graph.MarketData.plan('books').toString(),
    "select * exclude (executions, live, deltas, limits) where kind = 'book_event'",
  )
  assert.equal(
    graph.MarketData.plan('lifecycle', [], 'C-1').toString(),
    `select * exclude (${nested}) where crosscode = 'C-1' order by currunix`,
  )
  for (const view of enums.marketViews) {
    const crosscode = view === 'lifecycle' ? 'C-1' : undefined
    for (const lifts of [undefined, null, [], [lift], [new FieldPath(lift)]]) {
      const plan = graph.MarketData.plan(view, lifts, crosscode)
      assert.ok(plan instanceof Plan)
      const text = plan.toString()
      assert.equal(Plan.parse(text).toString(), text, view)
      assert.ok(Plan.parse(text).equals(plan), view)
    }
  }
})

test('a view is read by its spelling and only the lifecycle takes a crosscode', () => {
  assert.throws(() => graph.MarketData.plan('book-sides'), /\$\.view: expected one of orders, quotes, .*book_sides/)
  assert.throws(() => graph.MarketData.plan('lifecycle'), /\$\.crosscode: expected the crosscode of the chain/)
  assert.throws(() => graph.MarketData.plan('orders', [], 'C-1'), /\$\.crosscode: expected a crosscode only for the lifecycle view/)
  assert.throws(() => graph.MarketData.plan('orders', ['securityids[']), /invalid field path expression at byte 12/)
  // A refused view leaves the reader it was given unread.
  const reader = viewStream()
  assert.throws(() => graph.MarketData.applyView('nope', reader), /\$\.view/)
  assert.equal(reader.consumed, false)
})

test('each view keeps its own columns over a small stream', () => {
  const flat = rootNames().filter((name) => !NESTED.includes(name))
  for (const [view, kinds, rows] of [
    ['orders', ['order', 'order_event'], 5],
    ['quotes', ['quote', 'quote_event'], 2],
    ['executions', ['execution', 'execution_event'], 2],
  ]) {
    const table = viewed(view)
    assert.deepEqual(names(table), flat, view)
    assert.equal(table.numRows, rows, view)
    assert.ok([...table.getChild('kind')].every((kind) => kinds.includes(kind)), view)
  }

  // A trade is one row per execution, its own columns beside each.
  const trades = viewed('trades')
  assert.deepEqual(names(trades), [...flat, ...prefixed('executions', 'execution')])
  assert.equal(trades.numRows, 2)
  assert.deepEqual([...trades.getChild('crosscode')], ['T-1', 'T-1'])
  assert.deepEqual([...trades.getChild('execution.crosscode')], ['E-1', 'E-2'])

  // A book is two side rows beside its clocks, bid then ask.
  const sides = viewed('book_sides')
  assert.deepEqual(names(sides), ['currunix', 'snapunix', ...prefixed('bidside', 'side')])
  assert.equal(sides.numRows, 2)
  assert.deepEqual([...sides.getChild('side.side')], ['BUY', 'SELL'])

  const books = viewed('books')
  assert.deepEqual(names(books), rootNames().filter((name) => !['executions', 'live', 'deltas', 'limits'].includes(name)))
  assert.equal(books.numRows, 1)

  // A lifecycle is one chain in the order it happened, and needs its code.
  const chain = viewed('lifecycle', [], 'C-1')
  assert.deepEqual(names(chain), flat)
  assert.deepEqual([...chain.getChild('crosscode')], ['C-1', 'C-1'])
  // The stream held the later element first; the view answers the chain's
  // head, which follows nothing, then the element that follows it.
  assert.equal(chain.getChild('prevuuid').get(0), null)
  assert.notEqual(chain.getChild('prevuuid').get(1), null)
  assert.equal(viewed('lifecycle', undefined, 'NOWHERE').numRows, 0)
})

test('a lift reads one key of a root column and null where it is missing', () => {
  const lifts = ["securityids['ISIN'] as isin", "securityids['WKN'] as wkn"]
  const table = viewed('orders', lifts)
  assert.deepEqual(names(table).slice(-2), ['isin', 'wkn'])
  const codes = [...table.getChild('crosscode')]
  const isins = [...table.getChild('isin')]
  codes.forEach((code, at) => assert.equal(isins[at], code === 'O-5' ? ISIN : null, code))
  assert.equal(table.getChild('wkn').nullCount, table.numRows)
  // The key is read as it is stored: another case is another key.
  assert.equal(viewed('orders', ["securityids['isin'] as isin"]).getChild('isin').nullCount, 5)
  // A lift naming a column the root does not hold is refused where the
  // plan binds.
  assert.throws(() => viewed('orders', ["nothing['ISIN'] as isin"]), /nothing/)
  // Whatever `BatchReader.from` takes is a source: an Arrow JS table here.
  const table2 = graph.MarketData.applyView('quotes', graph.MarketData.arrowReader(leaves()).intoTable()).intoTable()
  assert.equal(table2.numRows, 2)
})
