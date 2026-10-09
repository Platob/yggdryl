'use strict'

// Candles, their options and the candle walk: `node/src/graph/candle.rs`,
// mirroring `rust/tests/graph/candle.rs`.

const assert = require('node:assert/strict')
const test = require('node:test')

const { Scalar, Timezone, graph } = require('yggdryl')

const SECOND = 1_000_000_000n
const MINUTE = 60n * SECOND
const HOUR = 60n * MINUTE
// `2026-03-29T00:00:00Z`, the day Europe/Zurich springs forward at `01:00Z`:
// its wall clock skips `02:00`-`03:00`.
const SPRING_DAY = 1_774_742_400n * SECOND
// `2026-10-25T00:00:00Z`, the day Europe/Zurich falls back at `01:00Z`: its
// wall clock reads `02:00`-`03:00` twice.
const FALL_DAY = 1_792_886_400n * SECOND
// `2026-01-05T10:00:00Z`.
const OFFSET_DAY = 1_767_607_200n * SECOND

function quote(unix, ticker, code, side, price, quantity, state = 'NEW') {
  return new graph.QuoteEvent(unix, { crosscode: code, ticker, side, price, quantity, state })
}

function books(operations) {
  return [...new graph.BookIterator(operations)]
}

function candles(operations, options) {
  return graph.candles(books(operations), options)
}

function emptyCandles(instants, options) {
  return graph.candles(instants.map((unix) => new graph.BookEvent(unix, 'ACME')), options)
}

function ohlc(open, high, low, close) {
  return { open, high, low, close }
}

function edges(folded) {
  return folded.map((candle) => [candle.start, candle.end, candle.books])
}

// The operations of `the OHLC of every reading over one minute`, four
// instants of one ticker.
function minute() {
  return [
    quote(10n * SECOND, 'ACME', 'B', 'BUY', '100', 5),
    quote(10n * SECOND, 'ACME', 'A', 'SELL', '103', 7),
    quote(20n * SECOND, 'ACME', 'B', 'BUY', '102', 5),
    quote(30n * SECOND, 'ACME', 'A', 'SELL', '102.5', 7),
    quote(30n * SECOND, 'ACME', 'B', 'BUY', '99', 5),
    quote(40n * SECOND, 'ACME', 'B', 'BUY', '101', 8),
    quote(40n * SECOND, 'ACME', 'A', 'SELL', '103.5', 9),
  ]
}

test('CandleOptions: an interval is a spelling or a count of nanoseconds, in a zone', () => {
  for (const [spelling, interval] of [
    ['30s', 30n * SECOND],
    ['1m', MINUTE],
    ['5m', 5n * MINUTE],
    ['1h', HOUR],
    ['1d', 24n * HOUR],
    ['1w', 7n * 24n * HOUR],
    ['250ms', 250_000_000n],
    ['7us', 7_000n],
    ['3ns', 3n],
  ]) {
    const options = new graph.CandleOptions(spelling)
    assert.equal(options.interval, interval, spelling)
    assert.equal(options.spelling, spelling)
    assert.ok(options.timezone.isUtc(), spelling)
    assert.equal(options.toString(), spelling)
  }
  // The widest unit that divides exactly is the one written back.
  assert.equal(new graph.CandleOptions(90n * SECOND).spelling, '90s')
  assert.equal(new graph.CandleOptions(120_000_000_000).spelling, '2m')
  assert.equal(new graph.CandleOptions(1_500_000_000).spelling, '1500ms')
  assert.equal(new graph.CandleOptions(14n * 24n * HOUR).spelling, '2w')

  // The zone is stated beside the interval, as a name or a `Timezone`.
  const zurich = new graph.CandleOptions('1h', 'Europe/Zurich')
  assert.equal(zurich.timezone.toString(), 'Europe/Zurich')
  assert.equal(zurich.spelling, '1h')
  assert.equal(zurich.toString(), '1h Europe/Zurich')
  assert.ok(zurich.equals(new graph.CandleOptions(HOUR, new Timezone('Europe/Zurich'))))
  assert.ok(!zurich.equals(new graph.CandleOptions('1h')))
  assert.ok(zurich.withTimezone('UTC').equals(new graph.CandleOptions('1h')))
  assert.ok(zurich.clone().equals(zurich))
  assert.equal(zurich.withTimezone('UTC').timezone.toString(), 'UTC')
  assert.equal(zurich.timezone.toString(), 'Europe/Zurich') // a `with` is a copy

  // Refusals: a spelling that is not a positive count and a unit, an
  // interval that is not positive, a fraction, a zone nobody knows.
  for (const refused of ['', 'm', '0s', '1x', '1.5m', '1 m', ' 1m', '-1m', '1M', '1min']) {
    assert.throws(
      () => new graph.CandleOptions(refused),
      new RegExp(`\\$\\.interval: .*${JSON.stringify(refused).replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}`),
      refused,
    )
  }
  for (const refused of [0, -1, 0n, -1n]) {
    assert.throws(() => new graph.CandleOptions(refused), /\$\.interval: expected a positive count of nanoseconds/)
  }
  assert.throws(() => new graph.CandleOptions(1.5), /interval must be a whole number/)
  // A zone this build has no rules for is a value the options hold; the walk
  // refuses it when it first buckets an instant.
  const unruled = new graph.CandleOptions('1h', 'Mars/Olympus')
  assert.equal(unruled.timezone.toString(), 'Mars/Olympus')
  assert.throws(
    () => graph.candles([new graph.BookEvent(1_000n, 'ACME')], unruled),
    /no rules for the time zone Mars\/Olympus/,
  )
})

test('the OHLC of every reading over one minute', () => {
  const operations = minute()
  assert.equal(books(operations).length, 4, 'one book per instant')
  const folded = candles(operations, '1m')
  assert.equal(folded.length, 1)
  const [candle] = folded
  assert.ok(candle instanceof graph.Candle)
  assert.equal(candle.crosscode, '3:0:ACME', 'the book\'s stored cross code')
  assert.equal(candle.ticker, 'ACME')
  assert.equal(candle.start, 0n)
  assert.equal(candle.end, MINUTE)
  assert.deepEqual(candle.bid, ohlc('100', '102', '99', '101'))
  assert.deepEqual(candle.ask, ohlc('103', '103.5', '102.5', '103.5'))
  assert.deepEqual(candle.mid, ohlc('101.5', '102.5', '100.75', '102.25'))
  assert.deepEqual(candle.spread, ohlc('3', '3.5', '1', '2.5'))
  assert.equal(candle.bidqty, '8')
  assert.equal(candle.askqty, '9')
  assert.equal(candle.books, 4)
  // A candle counts the books it folded; what traded is no book's.
  assert.equal('executions' in candle, false)
  assert.equal('volume' in candle, false)
  assert.equal(candle.toString(), 'Candle("3:0:ACME", start=0, end=60000000000)')
  // The same walk, spelled through its options or its own iterator.
  assert.ok(candles(operations, new graph.CandleOptions(MINUTE))[0].equals(candle))
  const walk = new graph.CandleIterator(new graph.BookIterator(operations), '1m')
  assert.equal(walk[Symbol.iterator](), walk)
  assert.ok(walk.options instanceof graph.CandleOptions)
  assert.equal(walk.options.spelling, '1m')
  const walked = [...walk]
  assert.equal(walked.length, 1)
  assert.ok(walked[0].equals(candle))
  assert.deepEqual(walk.next(), { value: undefined, done: true })
})

test('a one-sided book states no mid or spread', () => {
  const folded = candles(
    [quote(10n * SECOND, 'ACME', 'B', 'BUY', '100', 5), quote(20n * SECOND, 'ACME', 'B', 'BUY', '101', 6)],
    '1m',
  )
  assert.equal(folded.length, 1)
  const [candle] = folded
  assert.deepEqual(candle.bid, ohlc('100', '101', '100', '101'))
  assert.equal(candle.ask, null)
  assert.equal(candle.mid, null)
  assert.equal(candle.spread, null)
  assert.equal(candle.bidqty, '6')
  assert.equal(candle.askqty, null)
  assert.equal(candle.books, 2)
})

test('candles from delta books equal candles from the books rebuilt whole', () => {
  // With no grid a walk emits every book as a delta book, the first
  // following no book; a candle reads each book's top of book, which a
  // delta book states, so rebuilding them changes no candle.
  const operations = [
    quote(OFFSET_DAY + SECOND, 'ACME', 'B-1', 'BUY', '100', 10),
    quote(OFFSET_DAY + SECOND, 'ACME', 'A-1', 'SELL', '102', 5),
    quote(OFFSET_DAY + 20n * SECOND, 'ACME', 'B-2', 'BUY', '101', 4),
    quote(OFFSET_DAY + 70n * SECOND, 'ACME', 'A-1', 'SELL', '103', 5),
    quote(OFFSET_DAY + 90n * SECOND, 'ACME', 'B-1', 'BUY', '99', 1),
    quote(OFFSET_DAY + 130n * SECOND, 'ACME', 'A-2', 'SELL', '101.5', 2),
  ]
  const deltaBooks = books(operations)
  assert.ok(deltaBooks.every((book) => !book.isComplete))
  const whole = []
  for (const book of deltaBooks) {
    const previous = whole.at(-1) ?? graph.BookEvent.keyed(book.transunix, 'ACME')
    whole.push(book.withPrevious(previous))
  }
  assert.ok(whole.every((book) => book.isComplete))
  const folded = graph.candles(deltaBooks, '1m')
  const rebuilt = graph.candles(whole, '1m')
  assert.equal(folded.length, 3)
  assert.equal(rebuilt.length, 3)
  folded.forEach((candle, at) => assert.ok(candle.equals(rebuilt[at]), `candle ${at}`))
})

test('a reading a later book lacks keeps the earlier ones', () => {
  // The ask side empties at the second book: the ask, the mid and the
  // spread keep what the first book read, the touch quantities are the
  // last book's.
  const operations = [
    quote(10n * SECOND, 'ACME', 'B', 'BUY', '100', 5),
    quote(10n * SECOND, 'ACME', 'A', 'SELL', '102', 7),
    quote(20n * SECOND, 'ACME', 'A', 'SELL', '102', 0, 'CANCELED'),
  ]
  const folded = books(operations)
  assert.equal(folded.length, 2)
  assert.equal(folded[1].bestPrice('SELL'), null)
  const [candle] = candles(operations, '1m')
  assert.deepEqual(candle.ask, ohlc('102', '102', '102', '102'))
  assert.deepEqual(candle.mid, ohlc('101', '101', '101', '101'))
  assert.deepEqual(candle.spread, ohlc('2', '2', '2', '2'))
  assert.deepEqual(candle.bid, ohlc('100', '100', '100', '100'))
  assert.equal(candle.askqty, null)
  assert.equal(candle.bidqty, '5')
  assert.equal(candle.books, 2)
})

test('buckets close when the stream moves past them, and an empty one yields no candle', () => {
  const folded = candles(
    [
      quote(10n * SECOND, 'ACME', 'B', 'BUY', '100', 5),
      quote(59n * SECOND, 'ACME', 'B', 'BUY', '101', 5),
      quote(60n * SECOND, 'ACME', 'B', 'BUY', '102', 5),
      quote(200n * SECOND, 'ACME', 'B', 'BUY', '103', 5),
    ],
    '1m',
  )
  assert.deepEqual(edges(folded), [
    [0n, MINUTE, 2],
    [MINUTE, 2n * MINUTE, 1],
    [3n * MINUTE, 4n * MINUTE, 1],
  ])
  assert.equal(folded[0].bid.close, '101')
  assert.equal(folded[1].bid.open, '102')
})

test('two cross codes interleave and emit in cross-code order', () => {
  const folded = candles(
    [
      quote(10n * SECOND, 'IBM', 'IBM-B', 'BUY', '100', 5),
      quote(20n * SECOND, 'AAPL', 'AAPL-B', 'BUY', '200', 5),
      quote(30n * SECOND, 'IBM', 'IBM-B', 'BUY', '101', 5),
      quote(70n * SECOND, 'IBM', 'IBM-B', 'BUY', '102', 5),
      quote(80n * SECOND, 'AAPL', 'AAPL-B', 'BUY', '201', 5),
    ],
    '1m',
  )
  assert.deepEqual(
    folded.map((candle) => [candle.crosscode, candle.start, candle.books]),
    [
      ['3:0:AAPL', 0n, 1],
      ['3:0:IBM', 0n, 2],
      ['3:0:AAPL', MINUTE, 1],
      ['3:0:IBM', MINUTE, 1],
    ],
  )
  assert.deepEqual(folded[1].bid, ohlc('100', '101', '100', '101'))
  assert.equal(folded[1].ticker, 'IBM')
  // A book stating no ticker states none on its candle, and an empty symbol
  // keys the book by the ISIN that states none.
  const [bare] = graph.candles([new graph.BookEvent(10n * SECOND, '')], '1m')
  assert.equal(bare.ticker, null)
  assert.equal(bare.crosscode, '3:0:XX0000000000')
})

test('buckets align to the zone: a half-hour offset, a spring forward, a fall back', () => {
  // Asia/Kolkata is +05:30: `10:45Z` reads `16:15`, whose hour opens at
  // `16:00` local, `10:30Z`.
  const instants = [OFFSET_DAY + 45n * MINUTE, OFFSET_DAY + 89n * MINUTE]
  assert.deepEqual(edges(emptyCandles(instants, new graph.CandleOptions('1h', 'Asia/Kolkata'))), [
    [OFFSET_DAY + 30n * MINUTE, OFFSET_DAY + 90n * MINUTE, 2],
  ])
  assert.deepEqual(edges(emptyCandles(instants, '1h')), [
    [OFFSET_DAY, OFFSET_DAY + HOUR, 1],
    [OFFSET_DAY + HOUR, OFFSET_DAY + 2n * HOUR, 1],
  ])

  // Europe/Zurich, 2026-03-29: the hourly buckets abut in UTC and no candle
  // opens at the hour the zone skipped.
  const zurich = (spelling) => new graph.CandleOptions(spelling, 'Europe/Zurich')
  assert.deepEqual(
    edges(
      emptyCandles([SPRING_DAY + 30n * MINUTE, SPRING_DAY + 90n * MINUTE, SPRING_DAY + 150n * MINUTE], zurich('1h')),
    ),
    [
      [SPRING_DAY, SPRING_DAY + HOUR, 1],
      [SPRING_DAY + HOUR, SPRING_DAY + 2n * HOUR, 1],
      [SPRING_DAY + 2n * HOUR, SPRING_DAY + 3n * HOUR, 1],
    ],
  )
  // A daily candle opens at local midnight - `23:00Z` the day before - and
  // spans twenty-three hours on that day.
  const [daily] = emptyCandles([SPRING_DAY + 30n * MINUTE, SPRING_DAY + 20n * HOUR], zurich('1d'))
  assert.equal(daily.start, SPRING_DAY - HOUR)
  assert.equal(daily.end, SPRING_DAY + 22n * HOUR)
  assert.equal(daily.end - daily.start, 23n * HOUR)
  assert.equal(daily.books, 2)
  // 2026-10-25: the repeated local hour `02` is one two-hour bucket.
  assert.deepEqual(edges(emptyCandles([FALL_DAY + 30n * MINUTE, FALL_DAY + 90n * MINUTE], zurich('1h'))), [
    [FALL_DAY, FALL_DAY + 2n * HOUR, 2],
  ])
})

test('an unsorted stream is refused at the book and the walk fuses', () => {
  const unsorted = [
    new graph.BookEvent(2_000n, 'ACME'),
    new graph.BookEvent(2_000n, 'ACME'),
    new graph.BookEvent(1_000n, 'ACME'),
    new graph.BookEvent(3_000n, 'ACME'),
  ]
  const walk = new graph.CandleIterator(unsorted, '1m')
  assert.throws(
    () => walk.next(),
    /^Error: invalid record value at \$\.book\.transunix: expected an instant at or after 2000, got 1000$/,
  )
  assert.deepEqual(walk.next(), { value: undefined, done: true })
  assert.deepEqual(walk.next(), { value: undefined, done: true })
  assert.throws(() => graph.candles(unsorted, '1m'), /\$\.book\.transunix/)
  // An empty stream yields no candle.
  assert.deepEqual(graph.candles([], '1m'), [])
  assert.deepEqual([...new graph.CandleIterator(new Set(), '1h')], [])
})

test('a JavaScript failure is thrown as itself, and an item is a book or refused by the core', () => {
  function* items() {
    yield new graph.BookEvent(1_000n, 'ACME')
    throw new RangeError('the source gave up')
  }
  assert.throws(() => [...new graph.CandleIterator(items(), '1m')], {
    name: 'RangeError',
    message: 'the source gave up',
  })
  // An item of another class is refused by name, as every market stream
  // refuses one; a market leaf that is no book, bare or held by a
  // `MarketData`, is refused by the core's own narrowing, naming its kind.
  assert.throws(() => [...new graph.CandleIterator([1], '1m')], {
    name: 'TypeError',
    message: 'expected MarketData, a market leaf or a FixMsg, got number',
  })
  assert.throws(() => [...new graph.CandleIterator([new graph.Order()], '1m')], {
    name: 'Error',
    message: 'invalid record value at $.kind: expected book_event, got order',
  })
  // A `MarketData` holding a book folds as the book it is - the shape a
  // market-data table's rows arrive as - and one holding another leaf is
  // refused by the kind it holds.
  const held = books(minute()).map((book) => new graph.MarketData(book))
  const folded = graph.candles(held, '1m')
  const direct = candles(minute(), '1m')
  assert.equal(folded.length, direct.length)
  assert.ok(folded.every((candle, index) => candle.equals(direct[index])))
  assert.throws(() => [...new graph.CandleIterator([new graph.MarketData(new graph.Order())], '1m')], {
    name: 'Error',
    message: 'invalid record value at $.kind: expected book_event, got order',
  })
  assert.throws(() => new graph.CandleIterator(5, '1m'), /books must be an iterable/)
  assert.throws(() => new graph.CandleIterator([], '1x'), /\$\.interval/)
  assert.throws(() => graph.CandleIterator([], '1m'), /cannot be invoked without 'new'/)
})

test('a JavaScript failure follows the completed buckets, and the open one is dropped', () => {
  // As Python's walk and the core's: the failure crosses the walk as its
  // own error, so the bucket it cut short is dropped rather than closed as
  // though the stream had ended there.
  function* items() {
    yield new graph.BookEvent(10n * SECOND, 'ACME')
    yield new graph.BookEvent(70n * SECOND, 'ACME')
    throw new RangeError('the source gave up')
  }
  const walk = new graph.CandleIterator(items(), '1m')
  assert.deepEqual(edges([walk.next().value]), [[0n, MINUTE, 1]])
  assert.throws(() => walk.next(), { name: 'RangeError', message: 'the source gave up' })
  assert.deepEqual(walk.next(), { value: undefined, done: true }, 'the open bucket is dropped, not emitted')
  // A consumer streaming the candles sees the completed bucket alone.
  const seen = []
  assert.throws(
    () => {
      for (const candle of new graph.CandleIterator(items(), '1m')) seen.push(candle)
    },
    { name: 'RangeError', message: 'the source gave up' },
  )
  assert.deepEqual(edges(seen), [[0n, MINUTE, 1]])
  // An item the loader refuses is such a failure too, thrown as itself.
  const refused = []
  assert.throws(
    () => {
      for (const candle of new graph.CandleIterator([new graph.BookEvent(10n * SECOND, 'ACME'), 1], '1m')) {
        refused.push(candle)
      }
    },
    { name: 'TypeError', message: 'expected MarketData, a market leaf or a FixMsg, got number' },
  )
  assert.deepEqual(refused, [])
})

test('Candle.field declares every cell', () => {
  const field = graph.Candle.field()
  assert.equal(field.name, 'candle')
  assert.equal(field.nullable, false)
  const declared = Array.from(field.dtype, (child) => `${child.name}: ${child.dtype}${child.nullable ? '' : ' not null'}`)
  const expected = [
    'crosscode: utf8 not null',
    'ticker: utf8',
    'start: datetime64(ns,"UTC") not null',
    'end: datetime64(ns,"UTC") not null',
  ]
  for (const reading of ['bid', 'ask', 'mid', 'spread']) {
    for (const cell of ['open', 'high', 'low', 'close']) expected.push(`${reading}${cell}: decimal`)
  }
  expected.push('bidqty: decimal', 'askqty: decimal', 'books: uint64 not null')
  assert.deepEqual(declared, expected)
})

test('a candle round trips through its scalar and its JSON', () => {
  const [candle] = candles(minute(), '1m')
  const scalar = candle.intoScalar()
  assert.ok(scalar instanceof Scalar)
  assert.equal(scalar.length, 23)
  assert.ok(graph.Candle.fromScalar(scalar).equals(candle))
  assert.ok(candle.clone().equals(candle))
  assert.ok(!candle.equals(candles(minute(), '30s')[0]))

  // A plain object spelling the cells is read the way the column would
  // hold them: a text price, an integer instant, an absent name a null.
  const read = graph.Candle.fromScalar({
    crosscode: 'ACME',
    start: 0n,
    end: MINUTE,
    bidopen: '100.5',
    bidhigh: '100.5',
    bidlow: '100.5',
    bidclose: '100.5',
    books: 1,
  })
  assert.deepEqual(read.bid, ohlc('100.5', '100.5', '100.5', '100.5'))
  assert.equal(read.ticker, null)
  assert.equal(read.ask, null)
  assert.equal(read.start, 0n)
  assert.equal(read.end, MINUTE)
  assert.equal(read.books, 1)

  // The JSON is the flat row under `Candle.field()`: instants as ISO 8601
  // text, decimals as text, counts as numbers - what `JSON.stringify` writes
  // and `fromJSON` reads back, as the object or as its text.
  const json = candle.toJSON()
  assert.equal(json.crosscode, '3:0:ACME')
  assert.equal(json.start, '1970-01-01T00:00:00Z')
  assert.equal(json.end, '1970-01-01T00:01:00Z')
  assert.equal(json.bidopen, '100')
  assert.equal(json.midlow, '100.75')
  assert.equal(json.books, 4)
  assert.equal(Object.keys(json).length, 23)
  assert.ok(graph.Candle.fromJSON(json).equals(candle))
  assert.ok(graph.Candle.fromJSON(JSON.stringify(candle)).equals(candle))
  assert.deepEqual(JSON.parse(JSON.stringify([candle]))[0], json)
  // An empty book's candle: every optional cell null on the way out and in.
  const [bare] = graph.candles([new graph.BookEvent(10n * SECOND, '')], '1m')
  assert.equal(bare.toJSON().bidopen, null)
  assert.equal(bare.toJSON().ticker, null)
  assert.equal(bare.toJSON().crosscode, '3:0:XX0000000000')
  assert.ok(graph.Candle.fromJSON(bare.toJSON()).equals(bare))
  assert.ok(graph.Candle.fromScalar(bare.intoScalar()).equals(bare))

  // A scalar of another shape is refused under the candle.
  assert.throws(() => graph.Candle.fromScalar(1), /\$\.candle/)
  assert.throws(() => graph.Candle.fromScalar(null), /\$\.candle/)
  assert.throws(() => graph.Candle.fromScalar({ ...json, bidhigh: null }), /\$\.candle: .*four decimals or four nulls/)
  assert.throws(() => graph.Candle.fromScalar({ ...json, vwap: null }), /vwap/)
  assert.throws(() => graph.Candle.fromJSON({ ...json, crosscode: null }), /crosscode/)
  assert.throws(() => new graph.Candle(), /constructor/)
})

test('graph.candles takes a zone beside the interval, as Python does', () => {
  // `2026-01-05T10:00:00Z` is `11:00` in Zurich: an hourly bucket in that
  // zone opens on the local hour, which `10:00Z` also is, while a daily one
  // opens at local midnight - `23:00Z` the day before.
  const held = [new graph.BookEvent(OFFSET_DAY + 5n * SECOND, 'ACME')]
  const zoned = graph.candles(held, '1d', 'Europe/Zurich')
  assert.deepEqual(
    edges(zoned),
    edges(graph.candles(held, new graph.CandleOptions('1d', 'Europe/Zurich'))),
  )
  assert.equal(zoned[0].start, OFFSET_DAY - 11n * HOUR)
  assert.deepEqual(edges(graph.candles(held, '1d', new Timezone('Europe/Zurich'))), edges(zoned))
  // Without a zone the buckets are UTC's, and `undefined`/`null` is no zone.
  assert.equal(graph.candles(held, '1d')[0].start, OFFSET_DAY - 10n * HOUR)
  assert.equal(graph.candles(held, '1d', null)[0].start, OFFSET_DAY - 10n * HOUR)

  // A `CandleOptions` beside a zone is those buckets in that zone, and
  // without one keeps its own - as Python's `candles(books, options,
  // timezone)` and `CandleOptions(options, timezone)` read them.
  const daily = new graph.CandleOptions('1d')
  assert.deepEqual(edges(graph.candles(held, daily, 'Europe/Zurich')), edges(zoned))
  assert.deepEqual(edges(graph.candles(held, daily, new Timezone('Europe/Zurich'))), edges(zoned))
  assert.deepEqual(edges(graph.candles(held, new graph.CandleOptions('1d', 'Europe/Zurich'), null)), edges(zoned))
  assert.ok(new graph.CandleOptions(daily).equals(daily))
  assert.ok(new graph.CandleOptions(daily, 'Europe/Zurich').equals(new graph.CandleOptions('1d', 'Europe/Zurich')))
  const zurich = new graph.CandleOptions('1d', 'Europe/Zurich')
  assert.ok(new graph.CandleOptions(zurich).equals(zurich))
  assert.ok(new graph.CandleOptions(zurich, 'UTC').equals(daily))
})
