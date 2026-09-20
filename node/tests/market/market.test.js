'use strict'

// The market's products: what the market did, read out of what a venue
// said. Five values, one class each, and the codec doors that read them out
// of a stream of messages or a stream of batches of message rows.
//
// Every rule is the core's, pinned in `rust/tests/market/`; what these check
// is the crossing - the facts each product answers and how they cross, the
// readings the stated facts imply, the row round trip through `intoRow` and
// `fromRow`, the stream a JavaScript iterable feeds one message at a time,
// the statements door and the book iterator over it, the symbol a book is
// read under, the Arrow twin of every door, the refusals, the writers, and
// the three hops over the bridge's own capture.

const assert = require('node:assert/strict')
const path = require('node:path')
const test = require('node:test')

const { BatchReader, DataType, Field, IOBase, Scalar, TextOptions, fix, market } = require('yggdryl')

const SEED = path.join(__dirname, '..', '..', '..', 'config', 'fix')
// A second of a ULBridge's own capture, anonymized: the corpus
// `rust/tests/market/hops.rs` reads.
const CAPTURE = path.join(__dirname, '..', '..', '..', 'rust', 'tests', 'fix', 'ulbridge.log')
// The bridge's own row header, as the core spells it.
const ROWHEADER =
  String.raw`^(?P<timestamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}) \[(?P<msgthreadid>[1-9]\d*)` +
  String.raw`(?:-(?P<msgsessionid>[0-9a-f]{8}):(?P<msgctxid>[0-9a-f]{10}):(?P<msgseqnum>\d+))?\] ` +
  String.raw`\[(?P<msgpluginid>[^\]]+)\] \((?P<level>[A-Z]+)\) `
// The one intake clock the Rust suites read undated bytes under.
const SENDING = new DataType('datetime64(ns,"UTC")').scalar(1_704_190_530_000_000_000n)
// One second per grid step.
const STEP = 1_000_000_000n

let seedRegistry
function seed() {
  seedRegistry ??= fix.FixRegistry.fromHandle(SEED)
  return seedRegistry.clone()
}

/** A codec over the committed dictionary with one explicit intake clock. */
function fixedCodec(options = {}) {
  return new fix.FixCodec(seed(), { defaultSendingTime: SENDING, ...options })
}

/** The messages a fixture's lines parse into, in line order, before any walk. */
function parsed(codec, lines) {
  return lines.map((line) => codec.parseLine(Buffer.from(line)).next().value)
}

/** The messages as one stream of batches of FIX rows, the shape every Arrow door reads. */
function messageReader(codec, messages) {
  return codec.arrowReader(fix.schema(codec.registry), messages)
}

/** The bridge capture as the messages a text read answers. */
function captured(codec) {
  const options = new TextOptions()
  options.rowheader = ROWHEADER
  const messages = []
  for (const line of new IOBase(CAPTURE).readTextLines(options)) {
    for (const message of codec.parseTextLine(line)) messages.push(message)
  }
  return messages
}

/** One order's life: the placement, the acknowledgement, a partial fill and the fill. */
const LIFE = [
  '8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|59=0|15=USD|52=20260102-10:15:30.250|10=0|',
  '8=FIX.4.4|35=8|11=A1|37=O1|17=E0|150=0|39=0|38=100|151=100|14=0|55=AAPL|54=1|52=20260102-10:15:30.500|10=0|',
  '8=FIX.4.4|35=8|37=O1|17=E1|150=F|39=1|38=100|14=40|151=60|31=10.5|32=40|6=10.5|55=AAPL|54=1|52=20260102-10:15:31.100|10=0|',
  '8=FIX.4.4|35=8|37=O1|17=E2|150=F|39=2|38=100|14=100|151=0|31=10.5|32=60|6=10.5|55=AAPL|54=1|52=20260102-10:15:33.100|10=0|',
]

/** The order's acknowledgement, which fills nothing, and its two fills. */
const FILLS = [
  '8=FIX.4.4|35=8|11=A1|37=O1|17=E0|150=0|39=0|38=100|151=100|14=0|55=AAPL|54=1|15=USD|52=20260102-10:15:30.500|10=0|',
  '8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=F|39=1|38=100|14=40|151=60|31=10.5|32=40|30=XNAS|55=AAPL|54=1|15=USD|880=M1|52=20260102-10:15:31.100|10=0|',
  '8=FIX.4.4|35=8|11=A1|37=O1|17=E2|150=F|39=2|38=100|14=100|151=0|31=10.6|32=60|30=XNAS|55=AAPL|54=1|15=USD|880=M2|52=20260102-10:15:33.100|10=0|',
]

/** The buy side's report of a match with its parties, the sell side's, and a trade capture report. */
const REPORTS = [
  '8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=F|39=2|31=10.5|32=40|55=AAPL|54=1|15=USD|75=20260102|64=20260105|880=M1|453=2|448=FIRM|447=D|452=1|448=CLI-9|447=D|452=3|60=20260102-10:15:31.100|52=20260102-10:15:31.100|10=0|',
  '8=FIX.4.4|35=8|11=B7|37=O2|17=E9|150=F|39=2|31=10.5|32=40|55=AAPL|54=2|15=USD|75=20260102|880=M1|60=20260102-10:15:31.200|52=20260102-10:15:31.200|10=0|',
  '8=FIX.4.4|35=AE|571=T1|1003=TR1|31=11|32=5|55=AAPL|54=1|15=USD|75=20260102|60=20260102-10:15:40.000|52=20260102-10:15:40.000|10=0|',
]

/** A two-sided quote, its update and its cancel; then a one-sided quote of another identifier. */
const QUOTES = [
  '8=FIX.4.4|35=S|117=Q1|131=R1|55=AAPL|132=10.4|134=100|133=10.6|135=150|15=USD|62=20260102-10:16:00.000|52=20260102-10:15:30.250|10=0|',
  '8=FIX.4.4|35=S|117=Q1|55=AAPL|132=10.45|134=100|133=10.55|135=150|15=USD|62=20260102-10:16:00.000|52=20260102-10:15:31.250|10=0|',
  '8=FIX.4.4|35=Z|117=Q1|298=1|52=20260102-10:15:32.250|10=0|',
  '8=FIX.4.4|35=S|117=Q2|55=AAPL|54=1|132=10.3|134=50|15=USD|52=20260102-10:15:33.250|10=0|',
]

/** Two bids at one price and a third below, an offer, a two-sided quote, a fill, and another instrument. */
const MAKERS = [
  '8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|15=USD|52=20260102-10:15:30.100|10=0|',
  '8=FIX.4.4|35=D|11=A2|55=AAPL|54=1|38=50|44=10.5|15=USD|52=20260102-10:15:30.200|10=0|',
  '8=FIX.4.4|35=D|11=A3|55=AAPL|54=1|38=70|44=10.4|15=USD|52=20260102-10:15:30.300|10=0|',
  '8=FIX.4.4|35=D|11=S1|55=AAPL|54=2|38=80|44=10.7|15=USD|52=20260102-10:15:30.400|10=0|',
  '8=FIX.4.4|35=S|117=Q1|55=AAPL|132=10.45|134=30|133=10.6|135=40|15=USD|52=20260102-10:15:31.100|10=0|',
  '8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=F|39=2|38=100|14=100|151=0|31=10.5|32=100|55=AAPL|54=1|15=USD|52=20260102-10:15:32.100|10=0|',
  '8=FIX.4.4|35=D|11=M1|55=MSFT|54=1|38=10|44=400|15=USD|52=20260102-10:15:32.500|10=0|',
]

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/

/** The sixteen event facts a product answers, read once as getters and once as the event. */
function eventFacts(product) {
  const event = product.event()
  assert.match(product.curruuid, UUID)
  assert.match(product.crossuuid, UUID)
  assert.equal(typeof product.crosscode, 'string')
  assert.equal(typeof product.currhashcode, 'bigint')
  assert.equal(typeof product.crosshashcode, 'bigint')
  assert.equal(typeof product.currunix, 'bigint')
  assert.equal(typeof product.state, 'string')
  assert.equal(typeof product.seqnum, 'number')
  for (const name of [
    'curruuid', 'crossuuid', 'crosscode', 'currhashcode', 'crosshashcode', 'currunix', 'state',
    'seqnum', 'creaunix', 'expirunix', 'prevunix', 'prevuuid', 'snapunix',
  ]) {
    assert.equal(product[name], event[name], name)
  }
  for (const name of ['identifiers', 'parentuuids', 'srcuuids']) {
    assert.deepEqual(product[name], event[name], name)
  }
}

/** The row round trip and the public class protocol every product shares. */
function protocol(Product, product, name) {
  assert.ok(product instanceof Product)
  assert.equal(product.constructor, Product)
  const field = product instanceof market.Book ? Product.field(product.depth) : Product.field()
  assert.ok(field instanceof Field)
  assert.equal(field.name, name)
  assert.equal(field.indexOf('curruuid') < 16, true, 'the sixteen event columns open the row')
  // The row round trips as the row: the identity survives and the row read
  // back is the row written. The step before a statement - what the walk
  // filled - is no column, so a walked statement compares by its row.
  const row = product.intoRow()
  assert.ok(row instanceof Scalar)
  const back = Product.fromRow(field, row)
  assert.ok(back instanceof Product)
  assert.ok(back.intoRow().equals(row), 'the row states the product')
  assert.equal(back.curruuid, product.curruuid)
  assert.equal(back.seqnum, product.seqnum)
  assert.equal(back.prevuuid, product.prevuuid)
  assert.deepEqual(back.srcuuids, product.srcuuids)
  assert.ok(product.equals(product))
  assert.ok(back.equals(back))
  // A plain row crosses the same gate: whatever `Scalar.from` reads, typed
  // by the field in the core. The hashes and the instants are counts, so
  // they cross as bigints where a JSON document would have lost them.
  const cells = row.toJSON()
  for (const name of ['currhashcode', 'crosshashcode']) cells[field.indexOf(name)] = product[name]
  assert.ok(Product.fromRow(field, cells).intoRow().equals(row), 'a plain row is widened')
  // The renderings: a one-line summary, and the row's two documents, which
  // `JSON.stringify` composes.
  assert.match(product.toString(), new RegExp(`^${name.replace(/^\w/, (c) => c.toUpperCase())}\\(".*", \\d\\d[A-Z]+, \\d+\\)$`))
  const document = product.toJSON()
  assert.equal(document.field.name, name)
  assert.ok(Array.isArray(document.value))
  assert.equal(typeof JSON.stringify(product), 'string')
  // No constructor: a product is read, never built.
  assert.throws(() => new Product(), TypeError)
  assert.throws(() => Product(), TypeError)
}

test('an order is its placement and every report against it, chained', () => {
  const codec = fixedCodec()
  const messages = parsed(codec, LIFE)
  const orders = [...codec.orders(messages)]
  assert.equal(orders.length, 4, 'one statement per message about the order')
  for (const order of orders) eventFacts(order)

  // Every statement names the message it was read from, and nothing else.
  const chained = [...codec.lifecycle(messages)]
  for (const [at, order] of orders.entries()) {
    assert.deepEqual(order.srcuuids, [chained[at].curruuid], `order ${at}`)
    assert.equal(order.currunix, chained[at].currunix, `order ${at}`)
  }

  // The chain is the order's, opened under the client's identifier.
  const [placement, ack, partial, fill] = orders
  assert.equal(placement.crosscode, 'A1')
  assert.ok(orders.every((held) => held.crosscode === 'A1'))
  assert.ok(orders.every((held) => held.crossuuid === placement.crossuuid))
  assert.deepEqual(orders.map((held) => held.seqnum), [0, 1, 2, 3])
  assert.equal(placement.prevuuid, null)
  assert.equal(ack.prevuuid, placement.curruuid)
  assert.equal(partial.prevuuid, ack.curruuid)
  assert.equal(fill.prevuuid, partial.curruuid)
  assert.equal(fill.prevunix, partial.currunix)
  assert.deepEqual(fill.parentuuids, [placement.curruuid, ack.curruuid, partial.curruuid])
  // The names an order goes by accrue along the chain.
  assert.deepEqual(fill.identifiers, { clordid: 'A1', orderid: 'O1' })

  // The market facts, crossed as the event view crosses them: decimals as
  // text, an absent fact as null.
  assert.equal(placement.qty, '100')
  assert.equal(placement.px, '10.5')
  assert.equal(placement.tif, '0')
  assert.equal(placement.currency, 'USD')
  assert.equal(placement.unit, '')
  assert.equal(placement.symbolticker, 'AAPL')
  assert.equal(placement.side, 'BUY')
  assert.equal(placement.tradable, null)
  assert.equal(placement.stoppx, null)
  assert.equal(placement.avgpx, null)
  assert.equal(placement.isincode, null)
  assert.equal(placement.miccode, null)
  assert.equal(partial.cumqty, '40')
  assert.equal(partial.leavesqty, '60')
  assert.equal(partial.avgpx, '10.5')
  assert.equal(fill.leavesqty, '0')
  assert.equal(placement.event().px, '10.5')

  // The lifecycle folds forward: the state the chain reached, the earliest
  // creation, and a chain that ended.
  assert.deepEqual(orders.map((held) => held.state), ['00UNKNOWN', '20NEW', '40PARTFILL', '80FILLED'])
  assert.equal(fill.creaunix, placement.creaunix)
  assert.equal(placement.expirunix, null)
  assert.equal(placement.snapunix, null)

  // What the stated facts imply, read on every call: a bare noun is a
  // getter, a decimal crosses as its text.
  assert.equal(placement.ordtype, null, 'the placement states no type')
  assert.equal(placement.pricing, 'limit', 'a limit stated alone')
  assert.equal(placement.remaining, '100')
  assert.equal(placement.filled, '0')
  assert.equal(placement.filledRatio, '0')
  assert.equal(placement.notional, '1050')
  assert.equal(placement.isAlive, true)
  assert.equal(placement.isResting, true)
  assert.equal(partial.remaining, '60')
  assert.equal(partial.filled, '40')
  assert.equal(partial.filledRatio, '0.4')
  assert.equal(partial.isResting, true)
  assert.equal(fill.remaining, '0')
  assert.equal(fill.filled, '100')
  assert.equal(fill.filledRatio, '1')
  assert.equal(fill.isAlive, false, 'filled, the chain ended')
  assert.equal(fill.isResting, false)

  // Equality is by facts: a statement equals itself and no other.
  assert.ok(!placement.equals(ack))
  protocol(market.Order, partial, 'order')
  const field = market.Order.field()
  assert.equal(field.indexOf('px'), 16)
  assert.equal(field.indexOf('stoppx'), 33)
  assert.equal(field.indexOf('ordtype'), 34, 'the type follows the stop price')
})

test('an order prices by the type it states, else by what its limit and stop imply', () => {
  const codec = fixedCodec()
  const [stop, pegged, atMarket, stopLimit] = codec.orders(parsed(codec, [
    '8=FIX.4.4|35=D|11=X1|55=AAPL|54=1|38=10|40=3|99=9|15=USD|52=20260102-10:15:30.250|10=0|',
    '8=FIX.4.4|35=D|11=X2|55=AAPL|54=1|38=10|40=P|44=10|15=USD|52=20260102-10:15:30.350|10=0|',
    '8=FIX.4.4|35=D|11=X3|55=AAPL|54=1|38=10|40=1|15=USD|52=20260102-10:15:30.450|10=0|',
    '8=FIX.4.4|35=D|11=X4|55=AAPL|54=1|38=10|44=10|99=9|15=USD|52=20260102-10:15:30.550|10=0|',
  ]))
  assert.equal(stop.ordtype, '3', 'as the wire spelled it')
  assert.equal(stop.stoppx, '9')
  assert.equal(stop.pricing, 'stop')
  assert.equal(stop.isResting, false, 'a stop rests nothing until it triggers')
  assert.equal(pegged.ordtype, 'P')
  assert.equal(pegged.pricing, null, 'a type outside the four prices some other way')
  assert.equal(pegged.isResting, false)
  assert.equal(atMarket.pricing, 'market')
  assert.equal(atMarket.notional, null, 'worth nothing at no price')
  assert.equal(atMarket.isResting, false)
  assert.equal(stopLimit.ordtype, null)
  assert.equal(stopLimit.pricing, 'stoplimit', 'a limit and a stop, stated by no type')
  // Stated back, `OrdType(40)` is the type the order states, else its pricing.
  assert.equal(fix.FixMsg.fromOrder(codec, stop).byTag(40).asStr(), '3')
  assert.equal(fix.FixMsg.fromOrder(codec, stopLimit).byTag(40).asStr(), '4')
  // The type round trips through the row.
  const back = market.Order.fromRow(market.Order.field(), stop.intoRow())
  assert.equal(back.ordtype, '3')
  assert.equal(back.pricing, 'stop')
})

test('a report logged at two hops is one statement counted once', () => {
  const codec = fixedCodec()
  const lines = LIFE.slice()
  lines.splice(2, 0, LIFE[1])
  const messages = parsed(codec, lines)
  assert.equal([...codec.lifecycle(messages)].length, 5, 'the lifecycle yields the twin restated')
  const orders = [...codec.orders(messages)]
  assert.equal(orders.length, 4, 'the twin folded into the statement it restates')
  assert.deepEqual(orders.map((held) => held.seqnum), [0, 1, 2, 3])
  // Two hops of one message are one identity, so the fold names it once.
  const chained = [...codec.lifecycle(messages)]
  assert.equal(chained[1].curruuid, chained[2].curruuid, 'one identity however many hops logged it')
  assert.deepEqual(orders[1].srcuuids, [chained[1].curruuid])
})

test('the order stream is lazy, pulls one message at a time and throws what its source throws', () => {
  const codec = fixedCodec()
  const messages = parsed(codec, LIFE)

  // What is not iterable is refused before anything is pulled.
  assert.throws(() => codec.orders(42), TypeError)
  assert.throws(() => codec.books(42, 2, STEP), TypeError)
  // An item that is not a message ends the pull and throws in place of the
  // stream's end: the walk reads its source whole before it answers.
  const mixed = codec.orders([messages[0], '8=FIX.4.4|35=D|11=B|10=0|'])
  assert.equal(mixed.next().done, false)
  assert.throws(() => mixed.next(), TypeError)
  assert.equal(mixed.next().done, true)

  // A failure of the iterable throws as itself, once, and ends the stream.
  function* failing() {
    yield messages[0]
    throw new RangeError('the source broke')
  }
  const broken = codec.orders(failing())
  assert.equal(broken.next().done, false)
  assert.throws(() => broken.next(), RangeError)
  assert.equal(broken.next().done, true)

  // The stream is its own iterator, and exhaustion is fused.
  const stream = codec.orders(messages)
  assert.ok(stream instanceof market.Orders)
  assert.equal(stream[Symbol.iterator](), stream)
  const first = stream.next()
  assert.equal(first.done, false)
  assert.ok(first.value instanceof market.Order)
  assert.equal([...stream].length, LIFE.length - 1)
  assert.equal(stream.next().done, true)
  assert.throws(() => new market.Orders(), /no `constructor`/)
})

test('an execution is one fill, in the chain of the order it fills, and states back its report', () => {
  const codec = fixedCodec()
  const messages = parsed(codec, FILLS)
  const fills = [...codec.executions(messages)]
  assert.equal(fills.length, 2, 'the acknowledgement fills nothing')
  for (const fill of fills) eventFacts(fill)
  const [first, second] = fills
  assert.equal(first.px, '10.5')
  assert.equal(first.qty, '40')
  assert.equal(second.px, '10.6')
  assert.equal(second.qty, '60')
  assert.equal(first.side, 'BUY')
  assert.equal(first.currency, 'USD')
  assert.equal(first.unit, '')
  assert.equal(first.symbolticker, 'AAPL')
  assert.equal(first.miccode, 'XNAS')
  assert.equal(first.isincode, null)
  // Where the fill left its order, and what it was worth.
  assert.equal(first.isPartial, true)
  assert.equal(first.completes, false)
  assert.equal(first.notional, '420')
  assert.equal(first.isAlive, true)
  assert.equal(second.isPartial, false)
  assert.equal(second.completes, true)
  assert.equal(second.isAlive, false)
  assert.equal(second.notional, '636')
  // The execution's own identifier is among its names; the order it fills
  // is its chain, and the match is the trade's name.
  assert.deepEqual(first.identifiers, { clordid: 'A1', execid: 'E1', orderid: 'O1' })
  assert.equal(first.crosscode, 'O1')
  assert.equal(second.crossuuid, first.crossuuid)
  assert.equal(second.prevuuid, first.curruuid)
  assert.equal(second.seqnum, 1)
  const chained = [...codec.lifecycle(messages)]
  assert.deepEqual(first.srcuuids, [chained[1].curruuid])
  assert.deepEqual(second.srcuuids, [chained[2].curruuid])
  protocol(market.Execution, first, 'execution')
  assert.equal(market.Execution.field().indexOf('px'), 16)
  assert.equal(market.Execution.field().indexOf('miccode'), 27)

  // Stated back: `35=8`, `150=F`, the fill's own facts, naming the
  // execution as its one source; and what it stated is what reads back.
  const report = fix.FixMsg.fromExecution(codec, first)
  assert.ok(report instanceof fix.FixMsg)
  assert.equal(report.header().msgtype, '8')
  assert.equal(report.byTag(150).asStr(), 'F')
  assert.equal(report.byTag(17).asStr(), 'E1')
  assert.equal(report.byTag(37).asStr(), 'O1')
  assert.equal(report.byTag(30).asStr(), 'XNAS')
  assert.equal(report.px, '10.5')
  assert.equal(report.qty, '40')
  assert.deepEqual(report.srcuuids, [first.curruuid])
  const [again] = codec.executions([report])
  assert.equal(again.px, first.px)
  assert.equal(again.qty, first.qty)
  assert.equal(again.side, 'BUY')
})

test('an order states back its new order single', () => {
  const codec = fixedCodec()
  const [placement] = codec.orders(parsed(codec, LIFE))
  const single = fix.FixMsg.fromOrder(codec, placement)
  assert.equal(single.header().msgtype, 'D')
  assert.equal(single.byTag(11).asStr(), 'A1')
  assert.equal(single.byTag(40).asStr(), '2', 'a limit order')
  assert.equal(single.byTag(59).asStr(), '0')
  assert.equal(single.px, placement.px)
  assert.equal(single.qty, placement.qty)
  assert.equal(single.side, 'BUY')
  assert.deepEqual(single.srcuuids, [placement.curruuid])
  const [again] = codec.orders([single])
  assert.equal(again.qty, placement.qty)
  assert.equal(again.px, placement.px)
  assert.equal(again.crosscode, 'A1')
  // The static takes native values: a message is not an order.
  assert.throws(() => fix.FixMsg.fromOrder(codec, single), /instance of class/)
})

test('a trade is the matched quantity at its price with its parties and clocks', () => {
  const codec = fixedCodec()
  const messages = parsed(codec, REPORTS)
  const trades = [...codec.trades(messages)]
  assert.equal(trades.length, 3)
  for (const trade of trades) eventFacts(trade)
  const [buy, sell, captured] = trades
  assert.equal(buy.px, '10.5')
  assert.equal(buy.qty, '40')
  assert.equal(buy.side, 'BUY')
  assert.equal(sell.side, 'SELL')
  assert.equal(buy.currency, 'USD')
  assert.equal(buy.symbolticker, 'AAPL')
  // The clocks are days since the epoch, and the parties plain objects in
  // the order the report states them.
  assert.equal(buy.tradedate, 20_455, '2026-01-02 as days since the epoch')
  assert.equal(buy.settldate, 20_458)
  assert.equal(sell.settldate, null)
  assert.deepEqual(buy.parties, [
    { role: '1', id: 'FIRM', source: 'D' },
    { role: '3', id: 'CLI-9', source: 'D' },
  ])
  assert.deepEqual(sell.parties, [])
  // A party by its role, exactly as the report spells it, and the clocks'
  // implication.
  assert.deepEqual(buy.partyByRole('1'), { role: '1', id: 'FIRM', source: 'D' })
  assert.deepEqual(buy.partyByRole('3'), { role: '3', id: 'CLI-9', source: 'D' })
  assert.equal(buy.partyByRole('ExecutingFirm'), null, 'a role is matched as spelled')
  assert.equal(sell.partyByRole('1'), null)
  assert.equal(buy.settlementDays, 3, 'T+3')
  assert.equal(sell.settlementDays, null, 'no settlement date stated')
  assert.equal(buy.notional, '420')
  assert.equal(buy.isAlive, true, "40TRADE, so the match's chain stays open for the other side")
  // The chain is the match's: the two sides' reports share it.
  assert.equal(buy.crosscode, 'M1')
  assert.equal(sell.crossuuid, buy.crossuuid)
  assert.equal(sell.prevuuid, buy.curruuid)
  assert.equal(buy.identifiers.execid, 'E1')
  assert.equal('orderid' in buy.identifiers, false, "the order's names would chain every trade of the order as one")
  // A trade capture report names its own chain.
  assert.equal(captured.crosscode, 'TR1')
  assert.equal(captured.identifiers.tradereportid, 'T1')
  assert.equal(captured.qty, '5')
  assert.equal(captured.seqnum, 0)
  const chained = [...codec.lifecycle(messages)]
  for (const [at, trade] of trades.entries()) {
    assert.deepEqual(trade.srcuuids, [chained[at].curruuid], `trade ${at}`)
  }
  protocol(market.Trade, buy, 'trade')
  const field = market.Trade.field()
  assert.equal(field.indexOf('tradedate'), 28)
  assert.equal(field.indexOf('settldate'), 29)
  assert.equal(field.indexOf('parties'), 30)
  // No one message states a match, so the crate does not guess.
  assert.throws(() => fix.FixMsg.fromTrade(codec, buy), /trade.*does not guess/)
})

test('a quote is its lanes under its identifiers and its validity', () => {
  const codec = fixedCodec()
  const messages = parsed(codec, QUOTES)
  const quotes = [...codec.quotes(messages)]
  assert.equal(quotes.length, 4)
  for (const quote of quotes) eventFacts(quote)
  const [first, update, cancel, oneSided] = quotes
  assert.equal(first.bidpx, '10.4')
  assert.equal(first.bidqty, '100')
  assert.equal(first.askpx, '10.6')
  assert.equal(first.askqty, '150')
  assert.equal(first.bidcurrency, 'USD', "a lane is priced in the quote's currency")
  assert.equal(first.askcurrency, 'USD')
  assert.equal(first.bidunit, null)
  assert.equal(first.askunit, null)
  assert.equal(first.symbolticker, 'AAPL')
  assert.equal(first.expirunix, 1_767_348_960_000_000_000n)
  assert.equal(first.crosscode, 'Q1')
  assert.deepEqual(first.identifiers, { quoteid: 'Q1', quotereqid: 'R1' })
  // The lanes as lanes, the lane a side takes, and what the two imply.
  assert.deepEqual(first.bid, { px: '10.4', qty: '100' })
  assert.deepEqual(first.ask, { px: '10.6', qty: '150' })
  assert.deepEqual(first.lane('Buy'), first.bid, 'a side by its name')
  assert.deepEqual(first.lane('2'), first.ask, 'a side by its FIX code')
  assert.deepEqual(first.lane('SELL'), first.ask, 'a side by its stored value')
  assert.equal(first.lane('Cross'), null, 'a cross takes no lane')
  assert.throws(() => first.lane('Sideways'), /side/)
  assert.equal(first.isTwoSided, true)
  assert.equal(first.mid, '10.5')
  assert.equal(first.spread, '0.2')
  assert.equal(first.isAlive, true)
  // The update follows under the quote's identifier, and the cancel ends
  // the chain.
  assert.equal(update.prevuuid, first.curruuid)
  assert.equal(update.bidpx, '10.45')
  assert.equal(cancel.seqnum, 2)
  assert.equal(cancel.state, '90CANCELED')
  assert.equal(cancel.bidpx, null)
  assert.equal(cancel.askqty, null)
  assert.equal(cancel.bid, null)
  assert.equal(cancel.isTwoSided, false)
  assert.equal(cancel.mid, null)
  assert.equal(cancel.spread, null)
  assert.equal(cancel.isAlive, false, 'cancelled, the chain ended')
  // A one-sided quote fills the lane its side implies and no other.
  assert.equal(oneSided.crosscode, 'Q2')
  assert.equal(oneSided.seqnum, 0)
  assert.equal(oneSided.bidpx, '10.3')
  assert.equal(oneSided.askpx, null)
  assert.equal(oneSided.event().px, '10.3', 'about the bid it states')
  assert.deepEqual(oneSided.bid, { px: '10.3', qty: '50' })
  assert.equal(oneSided.ask, null)
  assert.equal(oneSided.lane('Sell'), null)
  assert.equal(oneSided.isTwoSided, false)
  assert.equal(oneSided.mid, null, 'no mid on a one-sided quote')
  const chained = [...codec.lifecycle(messages)]
  for (const [at, quote] of quotes.entries()) {
    assert.deepEqual(quote.srcuuids, [chained[at].curruuid], `quote ${at}`)
  }
  protocol(market.Quote, first, 'quote')
  assert.equal(market.Quote.field().indexOf('bidpx'), 16)
  assert.equal(market.Quote.field().indexOf('miccode'), 30)
  assert.throws(() => fix.FixMsg.fromQuote(codec, first), /quote.*does not guess/)
})

/** The bridge to the book's readings: what every book answers, typed. */
function bookReadings(book) {
  assert.equal(typeof book.px, 'string')
  assert.equal(typeof book.qty, 'string')
  assert.equal(typeof book.updates, 'number')
  assert.equal(typeof book.bidSize, 'string')
  assert.equal(typeof book.askSize, 'string')
  assert.equal(typeof book.bidCount, 'number')
  assert.equal(typeof book.askCount, 'number')
  assert.equal(typeof book.isTwoSided, 'boolean')
  assert.equal(typeof book.isLocked, 'boolean')
  assert.equal(typeof book.isCrossed, 'boolean')
  assert.equal(book.isAlive, true, 'a book has no lifecycle of its own')
  assert.equal(book.state, '00UNKNOWN')
  assert.deepEqual(book.identifiers, {}, 'a book goes by its symbol, which is its chain')
  assert.deepEqual(book.bestBid, book.bids[0] ?? null)
  assert.deepEqual(book.bestAsk, book.asks[0] ?? null)
  assert.equal(book.bidpx, book.bestBid?.px ?? null, 'the top of the ladder is the lane')
  assert.equal(book.bidqty, book.bestBid?.qty ?? null)
  assert.equal(book.askpx, book.bestAsk?.px ?? null)
  assert.equal(book.askqty, book.bestAsk?.qty ?? null)
  assert.equal(book.event().bidpx, book.bidpx)
  assert.equal(book.event().px, book.px, 'the mid is the price')
  assert.equal(book.mid ?? '0', book.px)
  assert.equal(book.isTwoSided, book.bestBid !== null && book.bestAsk !== null)
  assert.equal(book.bidCount, book.bids.reduce((count, level) => count + level.count, 0))
  assert.equal(book.askCount, book.asks.reduce((count, level) => count + level.count, 0))
}

test('one book per symbol per instant touched, to the declared depth, with its readings', () => {
  const codec = fixedCodec()
  const messages = parsed(codec, MAKERS)
  // The step unstated is no grid: one book per instant.
  const books = [...codec.books(messages, 2)]
  // Six instants of AAPL, one of MSFT.
  assert.equal(books.length, 7)
  for (const book of books) {
    eventFacts(book)
    bookReadings(book)
    assert.equal(book.snapunix, null, 'no grid')
    assert.equal(book.depth, 2)
  }
  const [first, second, third, fourth, quoted, filled, other] = books
  // The first instant: one bid, no offer, no mid.
  assert.equal(first.crosscode, 'AAPL')
  assert.equal(first.currunix, 1_767_348_930_100_000_000n)
  assert.deepEqual(first.bids, [{ px: '10.5', qty: '100', count: 1 }])
  assert.deepEqual(first.asks, [])
  assert.equal(first.bidpx, '10.5')
  assert.equal(first.askpx, null)
  assert.equal(first.px, '0', 'no mid on a one-sided book')
  assert.equal(first.qty, '100')
  assert.equal(first.mid, null)
  assert.equal(first.spread, null)
  assert.equal(first.spreadBps, null)
  assert.equal(first.microprice, null)
  assert.equal(first.imbalance, '1', 'a missing top counts as no size')
  assert.equal(first.isTwoSided, false)
  assert.equal(first.currency, 'USD')
  assert.equal(first.unit, '')
  assert.equal(first.symbolticker, 'AAPL')
  assert.equal(first.tradable, null)
  assert.equal(first.updates, 1)
  assert.equal(first.lastpx, null)
  assert.equal(first.cumqty, null)
  // Two bids at one price are one level, summed and counted.
  assert.deepEqual(second.bids, [{ px: '10.5', qty: '150', count: 2 }])
  assert.equal(second.bidCount, 2)
  assert.deepEqual(third.bids, [{ px: '10.5', qty: '150', count: 2 }, { px: '10.4', qty: '70', count: 1 }])
  assert.equal(third.bidSize, '220')
  assert.equal(third.askSize, '0')
  // The offer makes a mid, and the quote's lanes cut the ladder to the depth.
  assert.deepEqual(fourth.asks, [{ px: '10.7', qty: '80', count: 1 }])
  assert.equal(fourth.px, '10.6', 'the mid')
  assert.equal(fourth.qty, '300', 'resting on both ladders')
  assert.equal(fourth.spread, '0.2')
  assert.equal(fourth.isTwoSided, true)
  assert.deepEqual(quoted.bids, [{ px: '10.5', qty: '150', count: 2 }, { px: '10.45', qty: '30', count: 1 }])
  assert.deepEqual(quoted.asks, [{ px: '10.6', qty: '40', count: 1 }, { px: '10.7', qty: '80', count: 1 }])
  assert.deepEqual(quoted.bestBid, { px: '10.5', qty: '150', count: 2 })
  assert.deepEqual(quoted.bestAsk, { px: '10.6', qty: '40', count: 1 })
  assert.deepEqual(quoted.level('Buy', 1), { px: '10.45', qty: '30', count: 1 })
  assert.deepEqual(quoted.level('2', 0), quoted.bestAsk, 'a side by its FIX code')
  assert.equal(quoted.level('Sell', 2), null, 'past the ladder')
  assert.equal(quoted.level('Cross', 0), null, 'a cross takes no lane')
  assert.throws(() => quoted.level('Sideways', 0), /side/)
  assert.throws(() => quoted.level('Buy', -1), /index/)
  assert.equal(quoted.mid, '10.55')
  assert.equal(quoted.spread, '0.1')
  assert.equal(quoted.spreadBps, '94.786729857819905213')
  assert.equal(quoted.microprice, '10.578947368421052631')
  assert.equal(quoted.imbalance, '0.578947368421052631')
  assert.equal(quoted.imbalanceToDepth(1), quoted.imbalance)
  assert.equal(quoted.imbalanceToDepth(2), '0.2', '(180 - 120) / 300')
  assert.equal(quoted.imbalanceToDepth(0), null, 'no levels is no size')
  assert.throws(() => quoted.imbalanceToDepth(1.5), /levels/)
  assert.equal(quoted.bidSize, '180')
  assert.equal(quoted.askSize, '120')
  assert.equal(quoted.bidCount, 3)
  assert.equal(quoted.askCount, 2)
  assert.equal(quoted.isLocked, false)
  assert.equal(quoted.isCrossed, false)
  // The fill retires the first bid, and prints against the book.
  assert.deepEqual(filled.bids, [{ px: '10.5', qty: '50', count: 1 }, { px: '10.45', qty: '30', count: 1 }])
  assert.equal(filled.lastpx, '10.5')
  assert.equal(filled.lastqty, '100')
  assert.equal(filled.cumqty, '100')
  assert.equal(filled.avgpx, '10.5')
  assert.equal(filled.updates, 7, 'five makers, the order\'s fill and the print')
  // The chain is the symbol's, flat: each book follows the one before and
  // descends from it alone.
  assert.equal(second.prevuuid, first.curruuid)
  assert.deepEqual(books.slice(0, 6).map((held) => held.seqnum), [0, 1, 2, 3, 4, 5])
  assert.deepEqual(filled.parentuuids, [quoted.curruuid])
  assert.equal(filled.prevunix, quoted.currunix)
  // A book's sources are the statements it was read at, and what rested
  // any maker it retired.
  const chained = [...codec.lifecycle(messages)]
  assert.deepEqual(quoted.srcuuids, [chained[4].curruuid])
  assert.deepEqual(filled.srcuuids, [chained[0].curruuid, chained[5].curruuid].sort())
  // Another instrument is another chain.
  assert.equal(other.crosscode, 'MSFT')
  assert.equal(other.seqnum, 0)
  assert.deepEqual(other.bids, [{ px: '400', qty: '10', count: 1 }])
  assert.equal(other.updates, 1)

  // The row: the prints and the instrument, then the two ladders and the
  // count; the mid is re-derived from the ladders on the way back.
  protocol(market.Book, quoted, 'book')
  const field = market.Book.field(2)
  assert.equal(field.indexOf('lastpx'), 16)
  assert.equal(field.indexOf('tradable'), 20)
  assert.equal(field.indexOf('bids'), 30)
  assert.equal(field.indexOf('asks'), 31)
  assert.equal(field.indexOf('updates'), 32)
  const back = market.Book.fromRow(field, quoted.intoRow())
  assert.equal(back.px, quoted.px)
  assert.equal(back.microprice, quoted.microprice)
  assert.equal(back.updates, quoted.updates)
  assert.ok(!market.Book.field(3).equals(field), 'a depth is a declaration of the row')
  assert.throws(() => fix.FixMsg.fromBook(codec, first), /book.*does not guess/)
})

test('a grid reads one book per symbol per step at its closing state', () => {
  const codec = fixedCodec()
  const messages = parsed(codec, MAKERS)
  const books = [...codec.books(messages, 2, STEP)]
  // Three steps of AAPL, one of MSFT, in the order the steps closed.
  assert.equal(books.length, 4)
  for (const book of books) {
    eventFacts(book)
    bookReadings(book)
  }
  const [first, second, third, other] = books
  assert.equal(first.crosscode, 'AAPL')
  assert.equal(first.snapunix, 1_767_348_930_000_000_000n)
  assert.equal(first.currunix, 1_767_348_930_400_000_000n, 'dated at the last instant that moved it')
  assert.deepEqual(first.bids, [{ px: '10.5', qty: '150', count: 2 }, { px: '10.4', qty: '70', count: 1 }])
  assert.deepEqual(first.asks, [{ px: '10.7', qty: '80', count: 1 }])
  assert.equal(first.updates, 4)
  assert.equal(second.snapunix, 1_767_348_931_000_000_000n)
  assert.deepEqual(second.bids, [{ px: '10.5', qty: '150', count: 2 }, { px: '10.45', qty: '30', count: 1 }])
  assert.deepEqual(second.asks, [{ px: '10.6', qty: '40', count: 1 }, { px: '10.7', qty: '80', count: 1 }])
  assert.equal(second.prevuuid, first.curruuid, "the symbol's chain")
  assert.equal(second.seqnum, 1)
  assert.equal(third.snapunix, 1_767_348_932_000_000_000n)
  assert.deepEqual(third.bids, [{ px: '10.5', qty: '50', count: 1 }, { px: '10.45', qty: '30', count: 1 }])
  assert.equal(third.seqnum, 2)
  assert.equal(third.lastpx, '10.5')
  // The step's sources are every statement applied in it.
  const chained = [...codec.lifecycle(messages)]
  assert.deepEqual(first.srcuuids, chained.slice(0, 4).map((held) => held.curruuid).sort())
  assert.equal(other.crosscode, 'MSFT')
  assert.equal(other.snapunix, 1_767_348_932_000_000_000n)
  assert.deepEqual(other.bids, [{ px: '400', qty: '10', count: 1 }])

  // A step crosses as a bigint or as a whole number alike.
  assert.equal([...codec.books(messages, 2, Number(STEP))].length, 4)
  assert.equal([...codec.books(messages, 2, 0)].length, 7, 'a step of zero is no grid')
})

test('a depth of nothing and a step of nothing are refused before a message is pulled', () => {
  const codec = fixedCodec()
  const messages = parsed(codec, MAKERS)
  let pulled = 0
  function* counting() {
    for (const message of messages) {
      pulled += 1
      yield message
    }
  }
  assert.throws(() => codec.books(counting(), 0, STEP), /depth/)
  assert.throws(() => codec.books(counting(), 0), /depth/)
  assert.throws(() => codec.books(counting(), 2, -1n), /snapshot_ns/)
  assert.throws(() => codec.books(counting(), 2, -1), /snapshot_ns/)
  assert.throws(() => codec.books(counting(), 1.5, STEP), /depth/)
  assert.throws(() => codec.books(counting(), 2, 1.5), /snapshotNs/)
  assert.equal(pulled, 0)
  assert.throws(() => market.Book.field(0), /depth/)
  assert.throws(() => market.Book.field(-1), /depth/)
  assert.throws(() => market.Book.field(1.5), /depth/)
  // The Arrow twin refuses the same, and consumes its source as every
  // batch door consumes one.
  const source = messageReader(codec, messages)
  assert.throws(() => codec.booksArrowReader(source, 0, STEP), /depth/)
  assert.ok(source.consumed)
  assert.throws(() => codec.booksArrowReader(messageReader(codec, messages), 2, -1), /snapshot_ns/)
  assert.equal(codec.booksArrowReader(messageReader(codec, messages), 2).intoTable().numRows, 7, 'the step unstated is no grid')
})

test('every Arrow door agrees with its message door', () => {
  const codec = fixedCodec()
  const doors = [
    [market.Order, 'order', LIFE, (messages) => codec.orders(messages), (source) => codec.ordersArrowReader(source)],
    [market.Execution, 'execution', FILLS, (messages) => codec.executions(messages), (source) => codec.executionsArrowReader(source)],
    [market.Trade, 'trade', REPORTS, (messages) => codec.trades(messages), (source) => codec.tradesArrowReader(source)],
    [market.Quote, 'quote', QUOTES, (messages) => codec.quotes(messages), (source) => codec.quotesArrowReader(source)],
    [market.Book, 'book', MAKERS, (messages) => codec.books(messages, 2, STEP), (source) => codec.booksArrowReader(source, 2, STEP)],
    [market.Book, 'book', MAKERS, (messages) => codec.books(messages, 2), (source) => codec.booksArrowReader(source, 2)],
  ]
  for (const [Product, name, lines, door, twin] of doors) {
    const messages = parsed(codec, lines)
    const expected = [...door(messages)]
    const source = messageReader(codec, messages)
    const reader = twin(source)
    assert.ok(reader instanceof BatchReader, name)
    assert.ok(source.consumed, `${name}: the source is consumed`)
    const field = Product === market.Book ? Product.field(2) : Product.field()
    assert.ok(reader.field.equals(field), `${name}: the rows are the product's own`)
    const table = reader.intoTable()
    assert.equal(table.numRows, expected.length, name)
    assert.deepEqual(table.getChild('crosscode').toArray(), expected.map((held) => held.crosscode), name)
    assert.deepEqual(Array.from(table.getChild('currhashcode').toArray()), expected.map((held) => held.currhashcode), name)
    assert.deepEqual(Array.from(table.getChild('seqnum').toArray(), Number), expected.map((held) => held.seqnum), name)
    assert.deepEqual(table.getChild('state').toArray(), expected.map((held) => held.state), name)
    // The twin takes whatever `BatchReader.from` accepts: the table it
    // answered reads as the same rows.
    assert.equal(twin(messageReader(codec, messages).intoTable()).intoTable().numRows, expected.length, name)
  }
})

test('every product joins the message table on srcuuids to curruuid over the bridge capture', () => {
  const codec = fixedCodec({
    captureNames: ['timestamp', 'msgthreadid', 'msgsessionid', 'msgctxid', 'msgseqnum', 'msgpluginid', 'level'],
  })
  const messages = captured(codec)
  // The capture under the codec's own defaults, and the chains the walk
  // answers (`rust/tests/market/hops.rs`).
  assert.equal(messages.length, 79)
  // A product is read out of the walked message, whose identity the walk
  // settled again, so the message table it joins is the lifecycle's.
  const walked = [...codec.lifecycle(messages)]
  const known = new Set(walked.map((message) => message.curruuid))
  assert.equal(new Set(walked.map((message) => message.crossuuid)).size, 11)

  // Every source of every product is the identity of one message, every
  // product is one of at most as many as the messages it was read from,
  // and the count of statements and of identities is what the capture pins.
  function joins(products, [statements, identities]) {
    for (const product of products) {
      assert.ok(product.srcuuids.length > 0, 'a product names what it was read from')
      for (const source of product.srcuuids) assert.ok(known.has(source), 'a source that is no message')
    }
    assert.ok(products.length <= messages.length, 'products multiply no messages')
    const distinct = new Set(products.map((product) => product.curruuid))
    assert.ok(distinct.size <= known.size, 'products multiply no identities')
    assert.deepEqual([products.length, distinct.size], [statements, identities])
  }
  const orders = [...codec.orders(messages)]
  joins(orders, [18, 18])
  // An order's chain joins its own table: every predecessor is an order.
  const orderIdentities = new Set(orders.map((order) => order.curruuid))
  for (const order of orders) {
    if (order.prevuuid !== null) assert.ok(orderIdentities.has(order.prevuuid))
    for (const parent of order.parentuuids) assert.ok(orderIdentities.has(parent))
  }
  const executions = [...codec.executions(messages)]
  joins(executions, [14, 14])
  assert.ok(executions.length < orders.length, 'a fill is one report, an order every report')
  const trades = [...codec.trades(messages)]
  joins(trades, [19, 19])
  assert.ok(trades.length >= executions.length, 'every fill reports a trade, and hops add parties')
  const quotes = [...codec.quotes(messages)]
  joins(quotes, [0, 0])
  const books = [...codec.books(messages, 5, STEP)]
  joins(books, [10, 10])
  for (const book of books) {
    assert.notEqual(book.snapunix, null, 'every book is a snapshot of a step')
    assert.ok(book.bids.length <= 5 && book.asks.length <= 5)
    assert.deepEqual(book.identifiers, {})
  }
  // The statements door yields the orders and the fills, never a trade, and
  // the book iterator over it reads what the book door reads.
  const statements = [...codec.statements(messages)]
  assert.equal(statements.filter((held) => held instanceof market.Order).length, orders.length)
  assert.equal(statements.filter((held) => held instanceof market.Execution).length, executions.length)
  assert.equal(statements.filter((held) => held instanceof market.Quote).length, 0)
  assert.equal(statements.length, orders.length + executions.length)
  const iterated = [...new market.BookIterator(statements, 5, STEP)]
  assert.deepEqual(iterated.map((held) => held.curruuid), books.map((held) => held.curruuid))

  // And the Arrow twins answer the same over the same corpus.
  const rows = () => messageReader(codec, messages)
  assert.equal(codec.ordersArrowReader(rows()).intoTable().numRows, 18)
  assert.equal(codec.executionsArrowReader(rows()).intoTable().numRows, 14)
  assert.equal(codec.tradesArrowReader(rows()).intoTable().numRows, 19)
  assert.equal(codec.quotesArrowReader(rows()).intoTable().numRows, 0)
  assert.equal(codec.booksArrowReader(rows(), 5, STEP).intoTable().numRows, 10)
})

test('the namespace is frozen and holds the products, their streams, the symbol and the iterator alone', () => {
  assert.ok(Object.isFrozen(market))
  assert.deepEqual(Object.keys(market), [
    'Order', 'Quote', 'Execution', 'Trade', 'Book', 'Orders', 'Executions', 'Trades', 'Quotes', 'Books',
    'Statements', 'Symbol', 'BookIterator',
  ])
  const binding = require('yggdryl')
  for (const name of Object.keys(market)) {
    if (name !== 'Symbol') assert.equal(name in binding, false, name)
  }
  assert.equal(binding.Symbol, undefined, 'the global Symbol is not shadowed')
  for (const name of [
    'JsOrder', 'JsBooks', 'MarketLevelView', 'MarketPartyView', 'MarketLaneView', 'MarketSymbol',
    'JsMarketSymbol', 'JsStatements', 'JsBookIterator', '_bookIteratorNative',
  ]) {
    assert.equal(name in binding, false, name)
  }
  assert.equal(typeof fix.FixCodec.prototype._ordersNative, 'undefined')
  assert.equal(typeof fix.FixCodec.prototype._booksNative, 'undefined')
  assert.equal(typeof fix.FixCodec.prototype._statementsNative, 'undefined')
})
