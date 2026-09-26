'use strict'

// The one JSON renderer: `node/replay/json.js`.

const assert = require('node:assert/strict')
const test = require('node:test')

const arrow = require('apache-arrow')

const { BatchReader, graph } = require('yggdryl')

const {
  DEPTHS,
  INSTANT_COLUMNS,
  MARKETDATA_COLUMNS,
  bookJson,
  booksJson,
  decimalText,
  eventJson,
  leafFromJson,
  leafJson,
  refusalText,
  rowsOf,
  toJson,
} = require('../../replay/json.js')
const { books, synthetic, T0 } = require('../../replay/synthetic.js')

/** The last ALPHA book of the synthetic walk: a ladder, a market order, two executions. */
function alphaBook() {
  return books().filter((book) => book.crosscode === 'ALPHA').at(-1)
}

/** The native message `build` throws. */
function nativeRefusal(build) {
  try {
    build()
  } catch (error) {
    return error.message
  }
  assert.fail('expected the native constructor to refuse')
}

test('decimal text is exact: scaled, signed, trailing zeros trimmed', () => {
  assert.equal(decimalText(0n, 18), '0')
  assert.equal(decimalText(825n * 10n ** 17n, 18), '82.5')
  assert.equal(decimalText(-5n, 1), '-0.5')
  assert.equal(decimalText(1n, 18), '0.000000000000000001')
  assert.equal(decimalText(123_000n, 3), '123')
  assert.equal(decimalText(-(10n ** 38n) + 1n, 18), '-99999999999999999999.999999999999999999')
  assert.equal(decimalText(12n, -2), '1200')
})

test('the marketdata columns and its instants are the native field\'s', () => {
  const field = graph.MarketData.field()
  assert.equal(MARKETDATA_COLUMNS.length, field.fieldLen)
  assert.deepEqual(MARKETDATA_COLUMNS[0], { name: 'kind', dtype: 'utf8', nullable: false })
  assert.deepEqual(MARKETDATA_COLUMNS.find((column) => column.name === 'price'), {
    name: 'price',
    dtype: 'decimal',
    nullable: true,
  })
  assert.deepEqual([...INSTANT_COLUMNS], ['currunix', 'creaunix', 'execunix', 'recdunix', 'exprtime', 'prevunix', 'snapunix'])
})

test('rowsOf converts every column by its Arrow type, once per column', () => {
  const book = alphaBook()
  const { columns, rows } = rowsOf(graph.MarketData.arrowReader([book]))
  assert.deepEqual(columns, MARKETDATA_COLUMNS)
  assert.equal(rows.length, 1)
  const [row] = rows
  assert.deepEqual(Object.keys(row), columns.map((column) => column.name))
  // Utf8, an instant as its nanosecond text, a UUID as its hyphenated text.
  assert.equal(row.kind, 'book_event')
  assert.equal(row.currunix, String(T0 + 5_000_000n))
  assert.equal(row.snapunix, null)
  assert.equal(row.curruuid, book.curruuid)
  assert.match(row.curruuid, /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/)
  // A 64-bit integer is its decimal text; a boolean is a boolean.
  assert.equal(row.currhashcode, String(book.currhashcode))
  assert.equal(row.crossed, false)
  assert.equal(row.locked, false)
  // A decimal is its exact text: the ask's best is 82.5.
  assert.equal(row.askside.price, '82.5')
  assert.equal(row.spread, book.spread)
  // A map the book does not state is null; a list an array of its converted items.
  assert.equal(row.securityids, null)
  assert.equal(row.srcuuids, null)
  assert.equal(row.executions[0].state, '80FILLED')
  // A list of structs: the limits exactly as the native side answers them,
  // the market order's unpriced limit last.
  assert.deepEqual(row.bidside.limits, book.bid.limits)
  assert.deepEqual(row.bidside.limits.at(-1).price, null)
  assert.deepEqual(row.bidside.live.map((entry) => entry.curruuid), book.bid.live.map((entry) => entry.curruuid))
  assert.deepEqual(row.executions.map((entry) => entry.lastpx), ['82.5', '82.5'])
})

test('rowsOf reads the other widths exactly: a millisecond instant, a negative decimal256, a 32-bit integer', () => {
  const words = (units, bitWidth, rows) => {
    const stride = bitWidth / 32
    const out = new Uint32Array(stride * rows.length)
    rows.forEach((value, row) => {
      let held = BigInt.asUintN(bitWidth, value)
      for (let word = 0; word < stride; word += 1) {
        out[row * stride + word] = Number(held & 0xffff_ffffn)
        held >>= 32n
      }
    })
    return out
  }
  const big = [-15n * 10n ** 17n, 10n ** 60n]
  const table = new arrow.Table({
    at: arrow.vectorFromArray([1_700_000_000_123, null], new arrow.TimestampMillisecond()),
    big: arrow.makeVector(
      arrow.makeData({ type: new arrow.Decimal(18, 76, 256), length: 2, nullCount: 0, data: words(big, 256, big) }),
    ),
    count: arrow.vectorFromArray([7, -3], new arrow.Int32()),
  })
  const { columns, rows } = rowsOf(BatchReader.from(table))
  assert.deepEqual(columns.map((column) => column.name), ['at', 'big', 'count'])
  assert.deepEqual(rows, [
    { at: '1700000000123000000', big: '-1.5', count: 7 },
    { at: null, big: '1000000000000000000000000000000000000000000', count: -3 },
  ])
})

test('a crossed book states its negative spread as text', () => {
  const crossed = new graph.BookEvent(T0, 'X').withOperations([
    new graph.OrderEvent(T0, { crosscode: 'B', side: 'BUY', price: '103', quantity: 1, ticker: 'X' }),
    new graph.QuoteEvent(T0, { crosscode: 'A', side: 'SELL', price: '102.25', quantity: 1, ticker: 'X' }),
  ])
  const json = bookJson(crossed)
  assert.equal(json.spread, '-0.75')
  assert.equal(json.spread, crossed.spread)
  assert.equal(json.crossed, true)
})

test('bookJson: the sides in place of the lanes, and the readings no column holds', () => {
  const book = alphaBook()
  const json = bookJson(book)
  assert.equal(json.bidside, undefined)
  assert.equal(json.askside, undefined)
  // The side row, then its native depth per served level count and its length.
  assert.equal(json.bid.side, 'BUY')
  assert.equal(json.bid.price, book.bid.bestPrice)
  assert.equal(json.bid.quantity, book.bid.bestQuantity)
  assert.deepEqual(json.bid.limits, book.bid.limits)
  assert.deepEqual(Object.keys(json.bid.depth), DEPTHS.map(String))
  assert.deepEqual(json.bid.depth, { 1: book.bid.depth(1), 5: book.bid.depth(5), 10: book.bid.depth(10) })
  assert.equal(json.bid.depth['10'], '690')
  assert.equal(json.bid.length, book.bid.length)
  assert.equal(json.ask.limits[0].price, '82.5')
  assert.deepEqual(json.imbalance, { 1: book.imbalance(1), 5: book.imbalance(5), 10: book.imbalance(10) })
  assert.equal(json.imbalance['1'], '0.2')
  assert.equal(json.bboMidpoint, book.bboMidpoint)
  assert.equal(json.medianQuantity, book.medianQuantity)
  assert.equal(json.stableHash, String(book.stableHash()))
  assert.equal(json.isTick, false)
  // It survives JSON as it is: every value is already text, a number or a boolean.
  assert.deepEqual(JSON.parse(JSON.stringify(json)), json)
})

test('isTick is a grid step that changed nothing', () => {
  const walked = books(2)
  const served = booksJson(walked)
  assert.equal(served.length, walked.length)
  assert.deepEqual(
    served.filter((json) => json.isTick).map((json) => [json.crosscode, json.snapunix]),
    [
      ['ALPHA', String(T0 + 2_000_000n)],
      ['ALPHA', String(T0 + 4_000_000n)],
      ['ALPHA', String(T0 + 6_000_000n)],
    ],
  )
  // A grid step that carried a delta is no tick.
  assert.ok(served.some((json) => json.snapunix !== null && !json.isTick))
  assert.deepEqual(served.map((json) => json.stableHash), walked.map((book) => String(book.stableHash())))
})

test('leafJson: any leaf\'s row and its hash; a book as bookJson answers it', () => {
  const [first] = synthetic()
  const json = leafJson(first)
  assert.equal(json.kind, 'quote_event')
  assert.equal(json.curruuid, first.curruuid)
  assert.equal(json.price, '82')
  assert.equal(json.stableHash, String(first.stableHash()))
  const book = alphaBook()
  assert.deepEqual(leafJson(new graph.MarketData(book)), bookJson(book))
})

test("eventJson: the leaf's JSON beside the native text that rebuilds it", () => {
  const [first] = synthetic()
  const event = eventJson(first)
  assert.deepEqual({ ...event, native: undefined }, { ...leafJson(first), native: undefined })
  const rebuilt = graph.MarketData.fromJSON(event.native)
  assert.ok(rebuilt.equals(first))
  assert.equal(String(rebuilt.stableHash()), event.stableHash)
})

test('leafFromJson builds the native leaf, instants from their nanosecond text', () => {
  const leaf = leafFromJson({
    kind: 'order_event',
    currunix: String(T0),
    facts: { crosscode: 'N-1', ticker: 'ALPHA', side: 'BUY', price: '82.5', quantity: '10', exprtime: String(T0 + 500n) },
  })
  assert.ok(leaf instanceof graph.OrderEvent)
  assert.equal(leaf.currunix, T0)
  assert.equal(leaf.exprtime, T0 + 500n)
  assert.equal(leaf.price, '82.5')
  assert.equal(leafJson(leaf).exprtime, String(T0 + 500n))
  // A map is its [key, value] pairs, in the key order the native map holds.
  const identified = leafFromJson({
    kind: 'order_event',
    currunix: String(T0),
    facts: { securityids: { ISIN: 'US0378331005' }, metadata: { venue: 'X', account: 'A' } },
  })
  const keyed = leafJson(identified)
  // The CUSIP is the native package's derivation from the ISIN, in its key order.
  assert.deepEqual(keyed.securityids, [['CUSIP', '037833100'], ['ISIN', 'US0378331005']])
  assert.deepEqual(Object.fromEntries(keyed.securityids), identified.securityids)
  assert.deepEqual(keyed.metadata, [['account', 'A'], ['venue', 'X']])
  assert.ok(leafFromJson({ kind: 'quote', facts: { crosscode: 'Q' } }) instanceof graph.Quote)
  assert.ok(leafFromJson({ kind: 'execution_event', currunix: T0 }) instanceof graph.ExecutionEvent)
})

test('a map is its [key, value] pairs in the native order: integer-like keys and __proto__ keep theirs', () => {
  const pairs = [['31027', 'a'], ['7117', 'b'], ['__proto__', 'p'], ['venue', 'X']]
  const leaf = new graph.OrderEvent(T0, {
    crosscode: 'M',
    ticker: 'ALPHA',
    side: 'BUY',
    price: '82',
    quantity: 1,
    metadata: new Map(pairs),
  })
  // A plain object would hoist 7117 before 31027 and drop __proto__.
  const json = leafJson(leaf)
  assert.deepEqual(json.metadata, pairs)
  assert.deepEqual(JSON.parse(toJson(json)).metadata, pairs)
  assert.deepEqual(eventJson(leaf).metadata, pairs)
  // Every reading of a map column is the one reader: a view's rows too.
  const view = rowsOf(graph.MarketData.applyView('orders', graph.MarketData.arrowReader([leaf]), []))
  assert.deepEqual(view.rows[0].metadata, pairs)
  // The pairs are also what an inserted event may state, and state back exactly.
  const posted = leafFromJson({ kind: 'order_event', currunix: String(T0), facts: { crosscode: 'M', metadata: pairs } })
  assert.deepEqual(leafJson(posted).metadata, pairs)
  // Pairs out of the native key order are the constructor's to refuse.
  const reversed = [...pairs].reverse()
  assert.throws(
    () => leafFromJson({ kind: 'order_event', currunix: String(T0), facts: { metadata: reversed } }),
    { message: nativeRefusal(() => new graph.OrderEvent(T0, { metadata: new Map(reversed) })) },
  )
})

test('leafFromJson lets the native constructor refuse, verbatim', () => {
  const facts = { crosscode: 'N-1', price: 'abc' }
  const expected = nativeRefusal(() => new graph.OrderEvent(T0, facts))
  assert.match(expected, /\$\.price: expected an ISO decimal/)
  assert.throws(() => leafFromJson({ kind: 'order_event', currunix: String(T0), facts }), { message: expected })
  const side = nativeRefusal(() => new graph.QuoteEvent(T0, { side: 'UP' }))
  assert.throws(() => leafFromJson({ kind: 'quote_event', currunix: String(T0), facts: { side: 'UP' } }), {
    message: side,
  })
  assert.throws(() => leafFromJson({ kind: 'order_event', currunix: String(T0), facts: { bogus: 1 } }), {
    message: 'OrderEvent states no fact "bogus"',
  })
})

test('leafFromJson refuses what names no leaf before the constructor runs', () => {
  assert.throws(() => leafFromJson({ kind: 'book_event', currunix: String(T0) }), {
    name: 'TypeError',
    message:
      'expected a kind among order_event, quote_event, execution_event, order, quote, execution, got "book_event"',
  })
  assert.throws(() => leafFromJson({ kind: 'order_event', currunix: 1.5 }), {
    message: 'expected currunix as the decimal text of nanoseconds since the epoch, got 1.5',
  })
  assert.throws(() => leafFromJson({ kind: 'order_event' }), /expected currunix/)
  assert.throws(() => leafFromJson({ kind: 'order', currunix: '1' }), { message: 'an undated order states no currunix' })
  assert.throws(() => leafFromJson({ kind: 'order_event', currunix: '1', facts: [] }), /expected facts as an object/)
  assert.throws(() => leafFromJson({ kind: 'order_event', currunix: '1', facts: { metadata: [['a']] } }), {
    name: 'TypeError',
    message: 'metadata: expected a map as [key, value] pairs or an object keyed by text',
  })
  assert.throws(() => leafFromJson(null), /expected an event/)
})

test('refusalText is the message verbatim; toJson spells a bigint as text', () => {
  assert.equal(refusalText(new Error('$.price: nope')), '$.price: nope')
  assert.equal(refusalText('plain'), 'plain')
  assert.equal(toJson({ at: 1n << 62n, list: [1n] }), '{"at":"4611686018427387904","list":["1"]}')
})
