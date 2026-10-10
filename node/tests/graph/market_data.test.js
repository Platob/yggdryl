'use strict'

// `graph.MarketData` and its lifted Arrow doors: `node/src/graph/market_data.rs`.

const assert = require('node:assert/strict')
const test = require('node:test')

const arrow = require('apache-arrow')

const { BatchReader, Field, FieldPath, Identifier, MarketDataKind, Plan, Serie, enums, graph } = require('yggdryl')

/** The identifiers of a map by type, each value under its type. */
const kinds = (ids) => Object.fromEntries(ids.toArray().map((id) => [id.type, id.value]))


const CLOCK = 1_700_000_000_000_000_000n

// Every kind a leaf class is, in `MarketData.kinds()` order: all but `fix`,
// the FIX message a value holds whole, which its own test builds.
const LEAF_KINDS = graph.MarketData.kinds().filter((kind) => kind !== 'fix')

// One leaf of every kind, in `LEAF_KINDS` order.
function leaves() {
  const order = new graph.OrderEvent(CLOCK, { crosscode: 'O-1', side: 'BUYS', price: '101', quantity: 5, ticker: 'ACME', instcode: 'ACME' })
  const quote = new graph.QuoteEvent(CLOCK, { crosscode: 'Q-1', side: 'SELL', price: '102', quantity: 3, ticker: 'ACME', instcode: 'ACME' })
  const execution = new graph.ExecutionEvent(CLOCK + 1n, { crosscode: 'O-1', side: 'BUYS', lastpx: '101', lastqty: 5 })
  const trade = graph.TradeEvent.fromParts(new graph.ExecutionEvent(CLOCK, { crosscode: 'T-1', ticker: 'ACME' }), [
    new graph.ExecutionEvent(CLOCK, { crosscode: 'E-1', side: 'BUYS', lastpx: '1', lastqty: 1 }),
    new graph.ExecutionEvent(CLOCK, { crosscode: 'E-2', side: 'SELL', lastpx: '1', lastqty: 1 }),
  ])
  const book = new graph.BookEvent(CLOCK, 'ACME').withOperations([order, quote])
  return [
    new graph.Order({ crosscode: 'O-1', price: '101' }),
    new graph.Quote({ crosscode: 'Q-1' }),
    new graph.Execution({ crosscode: 'E-1', lastqty: 2 }),
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
  assert.deepEqual(wrapped.map((data) => data.kind), LEAF_KINDS)
  assert.deepEqual(wrapped.map((data) => data.isEvent), [...Array(3).fill(false), ...Array(6).fill(true)])
  // Each leaf stands under its market data category.
  assert.deepEqual(
    wrapped.map((data) => data.marketdatakind),
    ['ORDR', 'QUOT', 'EXEC', 'ORDR', 'QUOT', 'EXEC', 'TRAD', 'BOOK', 'BOOK'],
  )
  items.forEach((leaf, at) => {
    const data = wrapped[at]
    assert.ok(data.intoLeaf() instanceof leaf.constructor)
    assert.ok(data.intoLeaf().equals(leaf))
    assert.ok(new graph.MarketData(data).equals(data))
    // The element and market facts delegate to the leaf.
    for (const name of ['uuid', 'crosscode', 'price', 'side', 'hashcode', 'marketdatakind', 'instcode', 'isincode']) {
      assert.equal(data[name], leaf[name], name)
    }
  })
})

test('asLeaf borrows the leaf it is and null otherwise', () => {
  const order = leaves()[3]
  const data = new graph.MarketData(order)
  assert.ok(data.asOrderEvent().equals(order))
  assert.ok(data.intoLeaf() instanceof graph.OrderEvent)
  assert.equal('asBookSide' in data, false)
  for (const name of ['asOrder', 'asQuote', 'asExecution', 'asQuoteEvent',
    'asExecutionEvent', 'asTradeEvent', 'asBookEvent', 'asSnapshotEvent', 'asFix']) {
    assert.equal(data[name](), null, name)
  }
})

test('book answers the control of an operation or a snapshot', () => {
  const control = new graph.BookRef({ action: '0', scope: 'S' })
  assert.ok(new graph.MarketData(new graph.QuoteEvent(CLOCK, { book: control })).book.equals(control))
  assert.equal(new graph.MarketData(leaves()[8]).book.action, 'snapshot')
  assert.equal(new graph.MarketData(new graph.Order()).book, null)
})

test('anything but a leaf is refused', () => {
  assert.throws(() => new graph.MarketData(1), /expected MarketData, a market leaf or a FixMsg, got number/)
  assert.throws(() => new graph.MarketData(null), /expected MarketData, a market leaf or a FixMsg, got null/)
})

test('following crosses operation kinds and merging no variant', () => {
  const first = new graph.MarketData(new graph.OrderEvent(CLOCK, { crosscode: 'O-1', price: '1' }))
  const later = new graph.MarketData(new graph.OrderEvent(CLOCK + 1n, { crosscode: 'O-1', price: '2' }))
  const followed = later.withPrevious(first)
  assert.equal(followed.kind, 'order_event')
  assert.equal(followed.asOrderEvent().prevuuid, first.uuid)
  // An execution follows the order it fills across kinds, keeping its own.
  const execution = new graph.MarketData(new graph.ExecutionEvent(CLOCK + 1_000_000n, { crosscode: 'O-1' }))
  const fill = execution.withPrevious(first)
  assert.equal(fill.kind, 'execution_event')
  assert.equal(fill.asExecutionEvent().prevuuid, first.uuid)
  // A book follows no other variant, and a merge never crosses one.
  assert.equal(new graph.MarketData(new graph.BookEvent(CLOCK + 2n, 'X')).withPrevious(first), null)
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
      `MarketData(${data.uuid}, kind="${data.kind}", crosscode="${data.crosscode}")`,
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
  // The six identity columns lead, then the nine event columns, then the
  // market columns opening with the kind and its type.
  assert.equal(names[0], 'uuid')
  assert.equal(names[15], 'marketdatakind')
  assert.equal(field.fieldAt(15).dtype.id, 'marketdatakind')
  assert.equal(names[16], 'marketdatatype')
  assert.equal(field.fieldAt(16).dtype.id, 'marketdatatype')
  for (const name of ['transunix', 'price', 'isincode', 'fxrates', 'bidpx', 'askccy', 'identifiers', 'bookscope']) {
    assert.ok(names.includes(name), name)
  }
  // A1/A10: six identity, nine event, thirty-seven market - `instcode`
  // after `securityids` (D42) - and five operation columns, the three book
  // controls a book's delta replays by, and the six nested columns closing
  // the row: 66 in all.
  assert.equal(names.length, 6 + 9 + 37 + 5 + 3 + 6)
  assert.equal(names.indexOf('instcode'), names.indexOf('securityids') + 1)
  assert.deepEqual(names.slice(57, 60), ['bookscope', 'bookaction', 'bookposition'])
  assert.equal(field.fieldAt(59).dtype.id, 'uint32')
  assert.deepEqual(names.slice(60), NESTED)
  // When an element last executed is a market fact, stated among the
  // market columns, and the party ids an operation names an operation one.
  assert.ok(names.indexOf('execunix') > names.indexOf('state'))
  assert.equal(names.indexOf('execunix'), names.indexOf('miccode') + 1)
  assert.equal(names.indexOf('partyids'), names.indexOf('identifiers') + 1)
  for (const retired of ['kind', 'bid', 'ask', 'mdupdateaction', 'userids', 'marketoperationid',
    'bidside', 'askside', 'snapshotpartitions', 'live', 'limits', 'spread', 'crossed', 'locked']) {
    assert.equal(names.includes(retired), false, retired)
  }
})

test('arrowReader then fromArrowReader round trips every variant', () => {
  const items = leaves()
  const reader = graph.MarketData.arrowReader(items)
  assert.ok(reader instanceof BatchReader)
  assert.ok(reader.field.equals(graph.MarketData.field()))
  const back = drain(graph.MarketData.fromArrowReader(reader))
  assert.deepEqual(back.map((data) => data.kind), LEAF_KINDS)
  back.forEach((data, at) => {
    assert.ok(data instanceof graph.MarketData)
    assert.ok(data.intoLeaf().equals(items[at]), data.kind)
  })
})

test('the stream crosses a BatchReader round trip through IPC and Arrow JS', () => {
  const items = leaves()
  const bytes = graph.MarketData.arrowReader(items).intoIpc()
  const viaIpc = drain(graph.MarketData.fromArrowReader(BatchReader.fromIpc(bytes)))
  assert.equal(viaIpc.length, 9)
  viaIpc.forEach((data, at) => assert.ok(data.intoLeaf().equals(items[at]), data.kind))
  const table = graph.MarketData.arrowReader(items).intoTable()
  assert.equal(table.numRows, 9)
  // The column stores each kind's code.
  assert.deepEqual(
    [...table.getChild('marketdatakind')],
    ['ORDR', 'QUOT', 'EXEC', 'ORDR', 'QUOT', 'EXEC', 'TRAD', 'BOOK', 'BOOK'].map((name) => MarketDataKind[name]),
  )
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
    /expected MarketData, a market leaf or a FixMsg, got number/,
  )
  assert.throws(() => graph.MarketData.arrowReader(5), /items must be an iterable/)
})

test('a lifecycle-shaped batch reads into events', () => {
  // Event and operation columns in their own order, a foreign column, no
  // book column, the names folded, the types castable - the kind and the
  // side as their names - the door resolves what it knows once and ignores
  // the rest; a stated instant dates the leaf.
  const table = new arrow.Table({
    foreign: arrow.vectorFromArray([1, 2], new arrow.Int32()),
    // A row states its stored cross code: kind, side, then the base.
    CrossCode: arrow.vectorFromArray(['10:1:O-1', '8:1:O-1'], new arrow.Utf8()),
    MarketDataKind: arrow.vectorFromArray(['ORDR', 'EXEC'], new arrow.Utf8()),
    transunix: arrow.vectorFromArray([CLOCK, CLOCK + 1n], new arrow.Int64()),
    side: arrow.vectorFromArray(['BUYS', 'BUYS'], new arrow.Utf8()),
    price: arrow.vectorFromArray(['101', null], new arrow.Utf8()),
    lastqty: arrow.vectorFromArray([null, '5'], new arrow.Utf8()),
  })
  const [order, execution] = drain(graph.MarketData.fromArrowReader(BatchReader.from(table)))
    .map((data) => data.intoLeaf())
  assert.ok(order instanceof graph.OrderEvent)
  assert.ok(execution instanceof graph.ExecutionEvent)
  assert.equal(order.transunix, CLOCK)
  assert.equal(order.crosscode, '10:1:O-1')
  assert.equal(order.side, 'BUYS')
  assert.equal(order.price, '101')
  assert.equal(order.identifiers.length, 0)
  assert.equal(execution.crosscode, '8:1:O-1')
  assert.equal(execution.lastqty, '5')
  assert.equal(execution.transunix, CLOCK + 1n)
})

test('an identifier column built in Arrow JS is a map of keys to values and lands as the leaf\'s identifiers', () => {
  // A sorted map from each key's text - `src:type`, the type alone for the
  // base source - to its value.
  const identifiers = new arrow.Map_(
    new arrow.Field('entries', new arrow.Struct([
      new arrow.Field('key', new arrow.Utf8(), false),
      new arrow.Field('value', new arrow.Utf8(), false),
    ]), false),
    true,
  )
  const table = new arrow.Table({
    marketdatakind: arrow.vectorFromArray(['ORDR', 'ORDR'], new arrow.Utf8()),
    transunix: arrow.vectorFromArray([CLOCK, CLOCK + 1n], new arrow.Int64()),
    identifiers: arrow.vectorFromArray([new Map([['ullink:orderid', 'O-1']]), null], identifiers),
  })
  const [first, second] = drain(graph.MarketData.fromArrowReader(BatchReader.from(table)))
    .map((data) => data.intoLeaf())
  assert.deepEqual(kinds(first.identifiers), { orderid: 'O-1' })
  // A named source read back fills its type's base key: the map is closed.
  assert.deepEqual(first.identifiers.intoObject(), { orderid: 'O-1', 'ullink:orderid': 'O-1' })
  assert.equal(second.identifiers.length, 0)
})

test('fromArrowReader refuses by name and fuses', () => {
  const read = (columns) => drain(graph.MarketData.fromArrowReader(BatchReader.from(new arrow.Table(columns))))
  assert.throws(
    () => read({ transunix: arrow.vectorFromArray([CLOCK], new arrow.Int64()) }),
    /\$\[0\]\.marketdatakind: expected ORDR, QUOT, EXEC, TRAD or BOOK, got null/,
  )
  assert.throws(
    () => read({ marketdatakind: arrow.vectorFromArray(['nope'], new arrow.Utf8()) }),
    /marketdatakind/,
  )
  // A book row needs its instant (A2).
  const rows = graph.MarketData.fromArrowReader(BatchReader.from(new arrow.Table({
    marketdatakind: arrow.vectorFromArray(['ORDR', 'BOOK'], new arrow.Utf8()),
    transunix: arrow.vectorFromArray([CLOCK, null], new arrow.Int64()),
  })))
  assert.equal(rows.next().value.kind, 'order_event')
  assert.throws(() => rows.next(), /\$\[1\]/)
  assert.deepEqual(rows.next(), { value: undefined, done: true })
})

test('an undated row needs no clock', () => {
  const [data] = drain(graph.MarketData.fromArrowReader(BatchReader.from(new arrow.Table({
    marketdatakind: arrow.vectorFromArray(['ORDR'], new arrow.Utf8()),
    // A row states its stored cross code, which the door checks against the
    // one the row's kind and side derive: an order stating no side is `10:0:`.
    crosscode: arrow.vectorFromArray(['10:0:X'], new arrow.Utf8()),
  }))))
  assert.equal(data.kind, 'order')
  assert.equal(data.crosscode, '10:0:X')
  assert.ok(data.asOrder().equals(new graph.Order({ crosscode: 'X' })))
})

test('a book row states no sources while its delta and events rows do', () => {
  const lines = [1, 2, 3].map((n) => `018bcfe5-6800-7000-8000-00000000000${n}`)
  const facts = { price: '101', quantity: 1, ticker: 'ACME', instcode: 'ACME', state: 'NEW' }
  const order = new graph.OrderEvent(CLOCK, { crosscode: 'B-1', side: 'BUYS', srcuuids: [lines[0]], ...facts })
  const quote = new graph.QuoteEvent(CLOCK, { crosscode: 'Q-1', side: 'BUYS', srcuuids: [lines[1]], ...facts })
  const fill = new graph.ExecutionEvent(CLOCK, {
    crosscode: 'E-1', side: 'BUYS', lastqty: 1, ticker: 'ACME', instcode: 'ACME', srcuuids: [lines[2]],
  })
  const book = new graph.BookEvent(CLOCK, 'ACME').withOperations([order, quote, fill])
  // A book states no sources; the events it holds keep theirs.
  assert.deepEqual(book.srcuuids, [])
  assert.deepEqual(book.delta().map((entry) => entry.srcuuids), [[lines[0]], [lines[1]]])

  const table = graph.MarketData.arrowReader([book]).intoTable()
  assert.equal(table.getChild('srcuuids').get(0), null)
  const nested = (name) => [...table.getChild(name).get(0)].map((entry) => entry.toJSON().srcuuids)
  // An alive entry writes none: the delta row that applied it does.
  assert.deepEqual(nested('alive'), [null, null])
  assert.ok([...nested('delta'), ...nested('events')].every((sources) => sources !== null))

  // Read back, the book is the one written and states no source, nor do
  // its alive entries.
  const [read] = drain(graph.MarketData.fromArrowReader(graph.MarketData.arrowReader([book])))
  const held = read.asBookEvent()
  assert.ok(held)
  assert.deepEqual(held.uuid, book.uuid)
  assert.deepEqual(held.srcuuids, [])
  assert.deepEqual(held.alive().map((entry) => entry.uuid), book.alive().map((entry) => entry.uuid))
  assert.deepEqual(held.alive().map((entry) => entry.srcuuids), [[], []])

  // The delta and the events laid out of the book rows carry every source.
  const rows = () => Serie.fromArrowReader(graph.MarketData.arrowReader([book]))
  const sources = (serie) =>
    [...graph.MarketData.fromArrowReader(serie.intoArrowReader())].map((data) => data.srcuuids)
  assert.deepEqual(sources(graph.MarketData.deltaSerie(rows())), [[lines[0]], [lines[1]]])
  assert.deepEqual(sources(graph.MarketData.eventsSerie(rows())), [[lines[2]]])
})

// The named views over a `marketdata` stream: one plan each.
const NESTED = ['alive', 'delta', 'events', 'executions', 'bidlimits', 'asklimits']
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
    crosscode: 'O-5', side: 'BUYS', price: '100', ticker: 'ACME', securityids: [new Identifier('isin', ISIN)],
  })
  const first = new graph.OrderEvent(CLOCK + 10n, { crosscode: 'C-1', side: 'BUYS', price: '1' })
  const second = new graph.OrderEvent(CLOCK + 20n, { crosscode: 'C-1', side: 'BUYS', price: '2' }).withPrevious(first)
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
    'orders', 'quotes', 'executions', 'trades', 'books', 'lifecycle',
  ])
  assert.ok(Object.isFrozen(enums.marketViews))
})

test('every view is one plan whose text reads back', () => {
  const nested = NESTED.join(', ')
  const lift = "identifiers['clordid'] as clordid"
  assert.equal(
    graph.MarketData.plan('orders', [lift]).toString(),
    `select * exclude (${nested}), ${lift} where marketdatakind = 'ORDR'`,
  )
  assert.equal(
    graph.MarketData.plan('trades').toString(),
    `select * exclude (${nested}), unnest(executions) as execution where marketdatakind = 'TRAD'`,
  )
  // A book states its delta and its events - a complete one its alive
  // entries beside them - where a snapshot control states none.
  assert.equal(
    graph.MarketData.plan('BOOKS').toString(),
    "select * exclude (executions) where marketdatakind = 'BOOK' and (delta is not null or events is not null)",
  )
  assert.equal(
    graph.MarketData.plan('lifecycle', [], 'C-1').toString(),
    `select * exclude (${nested}) where crosscode = 'C-1' order by transunix`,
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
  assert.throws(() => graph.MarketData.plan('book-sides'), /\$\.view: expected one of orders, quotes, .*books, lifecycle/)
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
  for (const [view, kind, rows] of [
    ['orders', 'ORDR', 5],
    ['quotes', 'QUOT', 2],
    ['executions', 'EXEC', 2],
  ]) {
    const table = viewed(view)
    assert.deepEqual(names(table), flat, view)
    assert.equal(table.numRows, rows, view)
    assert.ok([...table.getChild('marketdatakind')].every((code) => code === MarketDataKind[kind]), view)
  }

  // A trade is one row per execution, its own columns beside each.
  const trades = viewed('trades')
  assert.deepEqual(names(trades), [...flat, ...prefixed('executions', 'execution')])
  assert.equal(trades.numRows, 2)
  assert.deepEqual([...trades.getChild('crosscode')], ['21:0:T-1', '21:0:T-1'])
  assert.deepEqual([...trades.getChild('execution.crosscode')], ['8:1:E-1', '8:2:E-2'])

  // A book keeps its alive entries, delta, events and levels nested; a
  // snapshot control, which states none of them, is no book row.
  const books = viewed('books')
  assert.deepEqual(names(books), rootNames().filter((name) => name !== 'executions'))
  assert.equal(books.numRows, 1)

  // A lifecycle is one chain in the order it happened, and needs its stored
  // code: the exact `{kind}:{side}:{base}` the chain's elements state.
  const chain = viewed('lifecycle', [], '10:1:C-1')
  assert.deepEqual(names(chain), flat)
  assert.deepEqual([...chain.getChild('crosscode')], ['10:1:C-1', '10:1:C-1'])
  // The stream held the later element first; the view answers the chain's
  // head, which follows nothing, then the element that follows it.
  assert.equal(chain.getChild('prevuuid').get(0), null)
  assert.notEqual(chain.getChild('prevuuid').get(1), null)
  assert.equal(viewed('lifecycle', undefined, 'NOWHERE').numRows, 0)
})

test('a lift reads one key of a root column and null where it is missing', () => {
  // An identifier column is a map from the key's text to its value: one
  // value by its key, the base key spelled as its type alone.
  const lifts = ["securityids['isin'] as isin", "securityids['wkn'] as wkn"]
  const table = viewed('orders', lifts)
  assert.deepEqual(names(table).slice(-2), ['isin', 'wkn'])
  const codes = [...table.getChild('crosscode')]
  const isins = [...table.getChild('isin')]
  codes.forEach((code, at) => assert.equal(isins[at], code === '10:1:O-5' ? ISIN : null, code))
  assert.equal(table.getChild('wkn').nullCount, table.numRows)
  // The key is read as it is stored, the lower-case text of a key: another
  // spelling is another key.
  assert.equal(viewed('orders', ["securityids['ISIN'] as isin"]).getChild('isin').nullCount, 5)
  // A lift naming a column the root does not hold is refused where the
  // plan binds.
  assert.throws(() => viewed('orders', ["nothing['ISIN'] as isin"]), /nothing/)
  // Whatever `BatchReader.from` takes is a source: an Arrow JS table here.
  const table2 = graph.MarketData.applyView('quotes', graph.MarketData.arrowReader(leaves()).intoTable()).intoTable()
  assert.equal(table2.numRows, 2)
})

test('a FixMsg is held whole and reports the leaves it expands to', () => {
  const { fix } = require('yggdryl')
  const seed = require('node:path').join(__dirname, '..', '..', '..', 'config', 'fix')
  const codec = new fix.FixCodec(fix.FixRegistry.fromHandle(seed), { excludeMsgtypes: [] })
  const order = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|52=20240102-10:15:30|11=A1|55=ACME|54=1|38=100|44=10.5|40=2|59=1|10=0|'))
  const data = new graph.MarketData(order)
  // Held whole: kind `fix`, its category the message's own `marketdatakind`.
  assert.equal(data.kind, 'fix')
  assert.equal(data.marketdatakind, order.marketdatakind)
  assert.equal(data.marketdatakind, 'ORDR')
  assert.ok(data.intoLeaf() instanceof fix.FixMsg)
  assert.ok(data.asFix() instanceof fix.FixMsg)
  assert.equal(data.asFix().timeinforce, 'GTC')
  assert.equal(data.asOrderEvent(), null)
  assert.equal(data.crosscode, order.crosscode)
  assert.equal(data.side, 'BUYS')
  // A lifted stream splits it into the leaves it reports, as `marketData` does.
  const leaves = order.marketData()
  const back = drain(graph.MarketData.fromArrowReader(graph.MarketData.arrowReader([order, data])))
  assert.deepEqual(back.map((row) => row.kind), ['order_event', 'order_event'])
  back.forEach((row) => assert.ok(row.equals(leaves[0])))
  assert.equal(back[0].asOrderEvent().timeinforce, 'GTC')
  // The timeinforce column stores the member's uint8 code.
  const table = graph.MarketData.arrowReader([order]).intoTable()
  assert.equal(String(table.getChild('timeinforce').type), 'Uint8')
  assert.equal(String(table.getChild('marketdatakind').type), 'Uint8')
  assert.equal(String(table.getChild('marketdatatype').type), 'Uint16')
  assert.deepEqual([...table.getChild('timeinforce')], [2])

  // A book message is one message holding one leaf per entry.
  const book = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=W|52=20240102-10:15:30|262=R|55=ACME|268=2|269=0|270=10|271=5|269=1|270=11|271=6|10=0|'))
  const held = new graph.MarketData(book)
  assert.equal(held.kind, 'fix')
  assert.equal(held.marketdatakind, 'BOOK')
  const entries = drain(graph.MarketData.fromArrowReader(graph.MarketData.arrowReader([held])))
  assert.deepEqual(entries.map((row) => row.kind), ['quote_event', 'quote_event'])
  // A book walk folds a held message's leaves as it folds the leaves themselves.
  const folded = [...new graph.BookIterator([order], 0)]
  const direct = [...new graph.BookIterator(leaves, 0)]
  assert.equal(folded.length, direct.length)
  folded.forEach((one, at) => assert.equal(one.hashcode, direct[at].hashcode))
})
