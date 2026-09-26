'use strict'

// The one JSON renderer: `node/replay/json.js`.

const assert = require('node:assert/strict')
const test = require('node:test')

const arrow = require('apache-arrow')

const { BatchReader, graph } = require('yggdryl')

const { DEPTHS, bookJson, booksJson, decimalText, refusalText, rowsOf, toJson } = require('../../replay/json.js')
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

test('rowsOf converts every column by its Arrow type, once per column', () => {
  const book = alphaBook()
  const { columns, rows } = rowsOf(graph.MarketData.arrowReader([book]))
  const field = graph.MarketData.field()
  assert.equal(columns.length, field.fieldLen)
  assert.deepEqual(columns[0], { name: 'kind', dtype: 'utf8', nullable: false })
  assert.deepEqual(columns.find((column) => column.name === 'price'), { name: 'price', dtype: 'decimal', nullable: true })
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

test('refusalText is the message verbatim; toJson spells a bigint as text', () => {
  assert.equal(refusalText(new Error('$.price: nope')), '$.price: nope')
  assert.equal(refusalText('plain'), 'plain')
  assert.equal(toJson({ at: 1n << 62n, list: [1n] }), '{"at":"4611686018427387904","list":["1"]}')
})
