'use strict'

// The operation leaves and `BookRef`: `node/src/graph/operation.rs`.

const assert = require('node:assert/strict')
const test = require('node:test')

const { Identifier, Identifiers, graph } = require('yggdryl')

/** The identifiers of a set by type, each value under its type. */
const kinds = (ids) => Object.fromEntries(ids.toArray().map((id) => [id.type, id.value]))


const CLOCK = 1_700_000_000_000_000_000n

// A buy order stating a fact from every one of the three column sets.
function orderEvent(facts = {}) {
  return new graph.OrderEvent(CLOCK, {
    crosscode: 'O-100',
    seqnum: 3,
    creaunix: CLOCK - 100_000_000_000n,
    recdunix: CLOCK - 50_000_000_000n,
    side: 'BUYS',
    price: '101',
    currency: 'USD',
    quantity: 5,
    ticker: 'ACME',
    timeinforce: '0',
    identifiers: new Identifiers([new Identifier('orderid', 'O-100')]),
    ...facts,
  })
}

test('an order event reads every fact back typed', () => {
  const event = orderEvent()
  assert.match(event.curruuid, /^[0-9a-f-]{36}$/)
  assert.match(event.crossuuid, /^[0-9a-f-]{36}$/)
  // The stored cross code is the kind, the side, then the base.
  assert.equal(event.crosscode, '10:1:O-100')
  assert.equal(typeof event.currhashcode, 'bigint')
  assert.equal(typeof event.crosshashcode, 'bigint')
  assert.deepEqual(event.srcuuids, [])
  assert.equal(event.currunix, CLOCK)
  assert.equal(event.state, 'UNKNOWN')
  assert.equal(event.seqnum, 3)
  assert.equal(event.creaunix, CLOCK - 100_000_000_000n)
  assert.equal(event.execunix, null)
  assert.equal(event.recdunix, CLOCK - 50_000_000_000n)
  assert.equal(event.exprunix, null)
  assert.equal(event.prevunix, null)
  assert.equal(event.prevuuid, null)
  assert.equal(event.snapunix, null)
  assert.equal(event.price, '101')
  assert.equal(event.currency, 'USD')
  assert.equal(event.quantity, '5')
  assert.equal(event.unit, '')
  assert.equal(event.side, 'BUYS')
  assert.equal(event.securityids.length, 0)
  assert.equal(event.isincode, null)
  assert.equal(event.cficode, null)
  assert.equal(event.miccode, null)
  for (const name of ['lastpx', 'lastqty', 'avgpx', 'cumqty', 'leavesqty', 'prevpx', 'prevqty',
    'spotrate', 'forwardpoints', 'askpx', 'askqty', 'askccy']) {
    assert.equal(event[name], null, name)
  }
  // A buying order's price and quantity are its side's bid facts.
  assert.equal(event.bidpx, '101')
  assert.equal(event.bidqty, '5')
  assert.equal(event.marketdatatype, 'UKNW')
  assert.equal(event.ticker, 'ACME')
  assert.deepEqual(event.metadata, {})
  assert.deepEqual(event.fxrates, {})
  // `'0'` is the `TimeInForce(59)` wire value of a day order, one spelling
  // of the member.
  assert.equal(event.timeinforce, 'DAY')
  assert.equal(event.tradable, null)
  assert.deepEqual(kinds(event.identifiers), { orderid: 'O-100' })
  // The party ids an operation names, by role: none stated here.
  assert.equal(event.partyids.length, 0)
  assert.equal(event.kind, 'order')
  assert.equal(event.marketdatakind, 'ORDR')
  // A1/A7: the retired facts answer nothing.
  for (const name of ['marketoperationid', 'userids', 'bid', 'ask']) {
    assert.equal(name in event, false, name)
  }
  assert.equal(event.isExecution, false)
  assert.equal(event.book, null)
  assert.equal(event.action, null)
  assert.equal(event.scope, '')
  assert.equal(event.isFullSnapshot, false)
})

test('the bid and ask facts and the rates read back as plain values', () => {
  const event = orderEvent({ bidpx: '100', bidqty: 2, bidccy: 'USD', askpx: 101n, askqty: '3', askccy: 'EUR' })
  assert.deepEqual(
    ['bidpx', 'bidqty', 'bidccy', 'askpx', 'askqty', 'askccy'].map((name) => event[name]),
    ['100', '2', 'USD', '101', '3', 'EUR'],
  )
  // Each marketdatakind answers the category of its leaf.
  assert.deepEqual(
    [new graph.Order(), new graph.QuoteEvent(CLOCK), new graph.Execution()].map((leaf) => leaf.marketdatakind),
    ['ORDR', 'QUOT', 'EXEC'],
  )
})

test('an undated element states no clock, state or chain', () => {
  const element = new graph.Order({ crosscode: 'O-1', price: '10', side: 'SELL', curruuid: undefined })
  assert.equal(element.crosscode, '10:2:O-1')
  assert.equal(element.kind, 'order')
  assert.equal(element.side, 'SELL')
  for (const name of ['currunix', 'state', 'seqnum', 'prevuuid']) {
    assert.throws(() => new graph.Order({ [name]: 1 }), /an undated element has no clock, state or chain/)
  }
  assert.equal('currunix' in element, false)
})

test('a derived identity is refused by name', () => {
  // `finalize` derives the four identities, so a stated one would be
  // overwritten: it is refused on the dated and the undated leaf alike.
  const uuid = '00000000-0000-8000-8000-000000000001'
  for (const [name, value] of [['curruuid', uuid], ['crossuuid', uuid], ['currhashcode', 7n], ['crosshashcode', 7n]]) {
    assert.throws(
      () => new graph.Order({ [name]: value }),
      new RegExp(`Order states no fact "${name}": an identity is derived`),
    )
    assert.throws(
      () => new graph.OrderEvent(CLOCK, { [name]: value }),
      new RegExp(`OrderEvent states no fact "${name}": an identity is derived`),
    )
  }
  // The two element facts `finalize` keeps are stated.
  assert.equal(new graph.Order({ crosscode: 'O-1', srcuuids: [] }).crosscode, '10:0:O-1')
})

test('currunix is stated once, as the first argument', () => {
  for (const facts of [{ currunix: 5 }, { currunix: 5n }, { CURRUNIX: 5 }]) {
    assert.throws(() => new graph.OrderEvent(1n, facts), /OrderEvent states currunix once, as its first argument/)
  }
})

test('a graph value that is no column value is refused by its key', () => {
  const control = new graph.BookRef({ action: 'NEW' })
  assert.throws(
    () => new graph.Order({ book: control }),
    { name: 'TypeError', message: 'Order states no fact "book": an undated element has no book control' },
  )
  assert.throws(
    () => new graph.Order({ foo: control }),
    { name: 'TypeError', message: 'Order states no fact "foo" as a BookRef' },
  )
  assert.throws(
    () => new graph.OrderEvent(CLOCK, { foo: new graph.Order() }),
    { name: 'TypeError', message: 'OrderEvent states no fact "foo" as a Order' },
  )
})

test('at dates an element and intoElement undates it', () => {
  const element = new graph.Quote({ crosscode: 'Q-1', side: 'SELL', price: '102' })
  const event = element.at(CLOCK)
  assert.ok(event instanceof graph.QuoteEvent)
  assert.equal(event.currunix, CLOCK)
  // A quote holds its two legs and is stored unsided: its side is a tag,
  // and the price it states on the offer is its ask leg.
  assert.equal(event.crosscode, '14:0:Q-1')
  assert.equal(event.price, element.price)
  assert.equal(event.askpx, '102')
  assert.equal(event.bidpx, null)
  const back = event.intoElement()
  assert.ok(back instanceof graph.Quote)
  assert.ok(back.equals(element))
  assert.equal(new graph.Execution().at(CLOCK).kind, 'execution')
  assert.equal(new graph.ExecutionEvent(CLOCK).intoElement().kind, 'execution')
  // An instant is a bigint or a whole number of at most 2^53.
  assert.equal(element.at(7).currunix, 7n)
  assert.throws(() => element.at(1.5), /unix/)
})

test('the kind is the type', () => {
  const same = { crosscode: 'X', price: '1' }
  assert.notEqual(new graph.OrderEvent(CLOCK, same).curruuid, new graph.QuoteEvent(CLOCK, same).curruuid)
  assert.equal(new graph.ExecutionEvent(CLOCK).isExecution, true)
  assert.equal(new graph.QuoteEvent(CLOCK).isExecution, false)
  assert.throws(() => new graph.Order(same).equals(new graph.Quote(same)))
})

test('book states the control facts', () => {
  const control = new graph.BookRef({ action: '0', scope: 'S', position: 1, entryPx: '1', entrySize: '2' })
  const event = new graph.QuoteEvent(CLOCK, { book: control, crosscode: 'Q' })
  assert.ok(event.book.equals(control))
  assert.equal(event.action, '0')
  assert.equal(event.scope, 'S')
  assert.equal(event.isFullSnapshot, false)
  assert.ok(event.withBook(control).equals(event))
  const plain = new graph.QuoteEvent(CLOCK, { crosscode: 'Q' })
  assert.ok(plain.withBook(control).equals(event))
  assert.equal(plain.book, null)
  assert.equal(new graph.OrderEvent(CLOCK, { book: new graph.BookRef({ action: 'snapshot' }) }).isFullSnapshot, true)
  assert.throws(() => new graph.OrderEvent(CLOCK, { book: 1 }), {
    name: 'TypeError',
    message: 'expected a BookRef for OrderEvent.book',
  })
})

test('an order follows the order it replaces', () => {
  const first = orderEvent({ seqnum: undefined })
  const later = new graph.OrderEvent(CLOCK + 1n, { crosscode: 'O-100', side: 'BUYS', price: '100', quantity: 4 })
  const followed = later.withPrevious(first)
  assert.notEqual(followed, null)
  assert.equal(followed.prevuuid, first.curruuid)
  assert.equal(followed.prevunix, first.currunix)
  // A later instant keeps its own place.
  assert.equal(followed.seqnum, 0)
  assert.equal(later.prevuuid, null) // immutable: the verb answered a new event
  assert.ok(first.isBefore(later) && later.isAfter(first))
  assert.ok(!first.isAfter(first))
  // Following refuses what it cannot follow, and a fold that changes
  // nothing answers null.
  assert.equal(first.withPrevious(later), null)
  assert.equal(first.mergeWith(first), null)
  assert.ok(followed.restating(followed).equals(followed))
})

test('following crosses no kind', () => {
  const order = orderEvent()
  const execution = new graph.ExecutionEvent(CLOCK + 1n, { crosscode: 'O-100' })
  assert.throws(() => execution.withPrevious(order))
})

for (const [name, build] of [
  ['OrderEvent', () => orderEvent()],
  ['QuoteEvent', () => new graph.QuoteEvent(CLOCK, { crosscode: 'Q', askpx: '1', askccy: 'EUR' })],
  ['ExecutionEvent', () => new graph.ExecutionEvent(CLOCK, { crosscode: 'E', lastpx: '1.5', lastqty: 3, state: 'FILLED' })],
  // A1: a row states the book scope alone - the update action and the
  // entry's position are walk-time facts, never row facts.
  ['QuoteEvent with a book', () => new graph.QuoteEvent(CLOCK, { book: new graph.BookRef({ scope: 'S' }) })],
  ['Order', () => new graph.Order({ crosscode: 'O', metadata: { k: 'v' } })],
  ['Quote', () => new graph.Quote()],
  ['Execution', () => new graph.Execution({ securityids: [new Identifier('isin', 'US0378331005')] })],
]) {
  test(`${name}: equals, stableHash, toString, clone and toJSON round trip`, () => {
    const leaf = build()
    const Class = leaf.constructor
    const twin = Class.fromJSON(leaf.toJSON())
    assert.ok(twin instanceof Class)
    assert.ok(twin.equals(leaf))
    assert.equal(twin.stableHash(), leaf.stableHash())
    assert.equal(leaf.stableHash(), leaf.currhashcode)
    assert.equal(twin.curruuid, leaf.curruuid)
    // `JSON.stringify` writes the same text, so a document carries it.
    assert.ok(Class.fromJSON(JSON.parse(JSON.stringify(leaf))).equals(leaf))
    assert.ok(leaf.clone().equals(leaf))
    assert.notEqual(leaf.clone(), leaf)
    assert.ok(leaf.toString().startsWith(`${Class.name}(${leaf.curruuid}`))
  })
}

test('fromJSON refuses text that is not one value of its class', () => {
  assert.throws(() => graph.OrderEvent.fromJSON('not base64!'), /expected base64 MarketData text/)
  assert.throws(() => graph.OrderEvent.fromJSON(new graph.Quote().toJSON()), /kind/)
})

test('BookRef: every slot reads back typed', () => {
  const control = new graph.BookRef({ action: '1', scope: 'S', position: 3, entryPx: '2', entrySize: '5' })
  assert.equal(control.action, '1')
  assert.equal(control.scope, 'S')
  assert.equal(control.position, 3)
  assert.equal(control.entryPx, '2')
  assert.equal(control.entrySize, '5')
  assert.ok(control.isStated() && control.isPartial())
  assert.ok(!control.isRangeDelete())
  assert.ok(!new graph.BookRef().isStated())
  assert.throws(() => new graph.BookRef({ action: '9' }), /unknown MdUpdateAction/)
})

test('BookRef: a slot crosses the same door a fact does', () => {
  const numbered = new graph.BookRef({ action: '1', position: 3, entryPx: 2, entrySize: 5n })
  assert.ok(numbered.equals(new graph.BookRef({ action: '1', position: 3, entryPx: '2', entrySize: '5' })))
  assert.throws(() => new graph.BookRef({ position: 1.5 }), /BookRef.position must be an unsigned 32-bit integer/)
  assert.throws(() => new graph.BookRef({ position: -1 }), /BookRef.position must be an unsigned 32-bit integer/)
  assert.throws(() => new graph.BookRef({ action: 1 }), /BookRef.action must be text/)
  assert.throws(() => new graph.BookRef({ entry_px: '1' }), /BookRef has no slot "entry_px"/)
  assert.ok(new graph.BookRef() instanceof graph.BookRef)
})

test('BookRef: equals, stableHash, toString, clone and toJSON round trip', () => {
  const control = new graph.BookRef({ action: 'snapshot', scope: 'S' })
  assert.ok(new graph.BookRef(control.toJSON()).equals(control))
  assert.equal(new graph.BookRef(control.toJSON()).stableHash(), control.stableHash())
  assert.notEqual(new graph.BookRef({ action: 'snapshot', scope: 'T' }).stableHash(), control.stableHash())
  assert.equal(typeof control.stableHash(), 'bigint')
  assert.ok(control.clone().equals(control))
  assert.equal(
    control.toString(),
    'BookRef(action="snapshot", scope="S", position=null, entryPx=null, entrySize=null)',
  )
})
