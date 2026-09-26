'use strict'

// The operation leaves, `Lane` and `BookRef`: `node/src/graph/operation.rs`.

const assert = require('node:assert/strict')
const test = require('node:test')

const { graph } = require('yggdryl')

const CLOCK = 1_700_000_000_000_000_000n

// A buy order stating a fact from every one of the three column sets.
function orderEvent(facts = {}) {
  return new graph.OrderEvent(CLOCK, {
    crosscode: 'O-100',
    seqnum: 3,
    creaunix: CLOCK - 100_000_000_000n,
    recdunix: CLOCK - 50_000_000_000n,
    side: 'BUY',
    price: '101',
    currency: 'USD',
    quantity: 5,
    ticker: 'ACME',
    tif: '0',
    altids: { ORDERID: 'O-100' },
    bid: new graph.Lane({ price: '101', quantity: '5' }),
    ...facts,
  })
}

test('an order event reads every fact back typed', () => {
  const event = orderEvent()
  assert.match(event.curruuid, /^[0-9a-f-]{36}$/)
  assert.match(event.crossuuid, /^[0-9a-f-]{36}$/)
  assert.equal(event.crosscode, 'O-100')
  assert.equal(typeof event.currhashcode, 'bigint')
  assert.equal(typeof event.crosshashcode, 'bigint')
  assert.deepEqual(event.srcuuids, [])
  assert.equal(event.currunix, CLOCK)
  assert.equal(event.state, '00UNKNOWN')
  assert.equal(event.seqnum, 3)
  assert.equal(event.creaunix, CLOCK - 100_000_000_000n)
  assert.equal(event.execunix, null)
  assert.equal(event.recdunix, CLOCK - 50_000_000_000n)
  assert.equal(event.exprtime, null)
  assert.equal(event.prevunix, null)
  assert.equal(event.prevuuid, null)
  assert.equal(event.snapunix, null)
  assert.equal(event.price, '101')
  assert.equal(event.currency, 'USD')
  assert.equal(event.quantity, '5')
  assert.equal(event.unit, '')
  assert.equal(event.side, 'BUY')
  assert.deepEqual(event.securityids, {})
  assert.equal(event.cficode, null)
  assert.equal(event.miccode, null)
  for (const name of ['lastpx', 'lastqty', 'avgpx', 'cumqty', 'leavesqty', 'prevpx', 'prevqty',
    'spotrate', 'forwardpoints']) {
    assert.equal(event[name], null, name)
  }
  assert.equal(event.ticker, 'ACME')
  assert.deepEqual(event.metadata, {})
  assert.equal(event.marketoperationid, null)
  assert.equal(event.tif, '0')
  assert.equal(event.tradable, null)
  assert.deepEqual(event.accountids, {})
  assert.deepEqual(event.userids, {})
  assert.deepEqual(event.altids, { ORDERID: 'O-100' })
  assert.equal(event.kind, 'order')
  assert.equal(event.isExecution, false)
  assert.equal(event.book, null)
  assert.equal(event.action, null)
  assert.equal(event.scope, '')
  assert.equal(event.isFullSnapshot, false)
})

test('bid answers a typed Lane, never a plain object', () => {
  const event = orderEvent()
  assert.ok(event.bid instanceof graph.Lane)
  assert.equal(event.ask, null)
  assert.equal(event.bid.price, event.price)
  assert.equal(event.bid.quantity, event.quantity)
  // A lane given as its own six slots is the same lane.
  assert.ok(orderEvent({ bid: { price: '101', quantity: '5' } }).equals(event))
})

test('an undated element states no clock, state or chain', () => {
  const element = new graph.Order({ crosscode: 'O-1', price: '10', side: 'SELL', curruuid: undefined })
  assert.equal(element.crosscode, 'O-1')
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
  assert.equal(new graph.Order({ crosscode: 'O-1', srcuuids: [] }).crosscode, 'O-1')
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
    () => new graph.OrderEvent(CLOCK, { foo: new graph.SnapshotPartition({ scope: 'S' }) }),
    { name: 'TypeError', message: 'OrderEvent states no fact "foo" as a SnapshotPartition' },
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
  assert.equal(event.crosscode, 'Q-1')
  assert.equal(event.price, element.price)
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
  const later = new graph.OrderEvent(CLOCK + 1n, { crosscode: 'O-100', side: 'BUY', price: '100', quantity: 4 })
  const followed = later.withPrevious(first)
  assert.notEqual(followed, null)
  assert.equal(followed.prevuuid, first.curruuid)
  assert.equal(followed.prevunix, first.currunix)
  assert.equal(followed.seqnum, 1)
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
  ['QuoteEvent', () => new graph.QuoteEvent(CLOCK, { crosscode: 'Q', ask: new graph.Lane({ price: '1', currency: 'EUR' }) })],
  ['ExecutionEvent', () => new graph.ExecutionEvent(CLOCK, { crosscode: 'E', lastpx: '1.5', lastqty: 3, state: 'FILLED' })],
  ['QuoteEvent with a book', () => new graph.QuoteEvent(CLOCK, { book: new graph.BookRef({ action: '2', scope: 'S', position: 4 }) })],
  ['Order', () => new graph.Order({ crosscode: 'O', metadata: { k: 'v' } })],
  ['Quote', () => new graph.Quote()],
  ['Execution', () => new graph.Execution({ securityids: { ISIN: 'US0378331005' } })],
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

test('Lane: every slot reads back typed', () => {
  const lane = new graph.Lane({
    price: '1.5', spotrate: '1.4', forwardpoints: '0.1', currency: 'EUR', quantity: '2', unit: 'SHARES',
  })
  assert.equal(lane.price, '1.5')
  assert.equal(lane.spotrate, '1.4')
  assert.equal(lane.forwardpoints, '0.1')
  assert.equal(lane.currency, 'EUR')
  assert.equal(lane.quantity, '2')
  assert.equal(lane.unit, 'SHARES')
  assert.ok(lane.isStated())
})

test('Lane: undefined and null both state nothing', () => {
  assert.ok(new graph.Lane().equals(new graph.Lane({ price: null })))
  assert.ok(new graph.Lane({ price: undefined }).equals(new graph.Lane()))
  assert.ok(!new graph.Lane().isStated())
  assert.equal(new graph.Lane().price, null)
  assert.throws(() => new graph.Lane({ price: 'x' }))
})

test('Lane: a slot crosses the same door a fact does', () => {
  // A number or a bigint is the decimal it states, as under an operation's
  // facts; a fraction is refused the same way there and here.
  assert.ok(new graph.Lane({ price: 1, quantity: 2n }).equals(new graph.Lane({ price: '1', quantity: '2' })))
  assert.ok(new graph.Order({ bid: new graph.Lane({ price: 1 }) }).equals(new graph.Order({ bid: { price: 1 } })))
  assert.throws(() => new graph.Lane({ price: 1.5 }), /\$\.lane\.price: expected a decimal representable at scale 18 within 38 digits, got f64/)
  assert.throws(() => new graph.Order({ price: 1.5 }), /\$\.price: expected a decimal representable at scale 18 within 38 digits, got f64/)
  assert.throws(() => new graph.Lane({ bogus: 1 }), /Lane has no slot "bogus"/)
  assert.ok(new graph.Lane() instanceof graph.Lane)
})

test('Lane: equals, stableHash, toString, clone and toJSON round trip', () => {
  const lane = new graph.Lane({ price: '1', currency: 'USD' })
  assert.ok(new graph.Lane(lane.toJSON()).equals(lane))
  assert.equal(new graph.Lane(lane.toJSON()).stableHash(), lane.stableHash())
  assert.ok(lane.clone().equals(lane))
  assert.equal(
    lane.toString(),
    'Lane(price="1", spotrate=null, forwardpoints=null, currency="USD", quantity=null, unit=null)',
  )
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
