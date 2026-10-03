'use strict'

// Pins node/src/window_serie.rs: a window over a serie that reads and writes
// through the serie at each call, every index window-relative. Each case
// mirrors rust/tests/root/window_serie.rs by name.

const assert = require('node:assert/strict')
const test = require('node:test')

const {
  Field,
  Scalar,
  Serie,
  SerieReader,
  SpillOptions,
  WindowSerie,
  StructSerie,
} = require('yggdryl')

// Prices as an int64 column, `null` an absent row.
const column = (values) =>
  Serie.fromScalars(new Field('price', 'int64', values.includes(null)), values)
const run = (values) => new Serie(values)

test('a window reads through the serie, window-relative, on both leaves', () => {
  for (const serie of [column([1, 2, null, 4, 5, 6]), run([1, 2, null, 4, 5, 6])]) {
    const window = serie.window(1, 3)
    assert.ok(window instanceof WindowSerie)
    assert.deepEqual([window.length, window.offset, window.isEmpty()], [3, 1, false])
    // The window holds the very serie it was taken from.
    assert.strictEqual(window.serie, serie)
    assert.equal(String(window.field), String(serie.field))
    assert.equal(window.scalar(0).asJs(), 2)
    assert.equal(window.isNull(1), true)
    assert.equal(window.at(2).asJs(), 4)
    assert.equal(window.at(3), null)
    assert.equal(window.nullCount(), 1)
    assert.deepEqual(
      window.rows().map((row) => row.asJs()),
      [2, null, 4],
    )
    assert.deepEqual(window.asJs(), [2, null, 4])
    assert.deepEqual(JSON.parse(JSON.stringify(window)), [2, null, 4])
    assert.equal([...window].length, 3)
    assert.equal([...window][2].asJs(), 4)
    assert.equal(
      window.dtype.toString(),
      'serie(field("item",int64,nullable=true,metadata={}))',
    )
    assert.ok(window.memorySize() > 0)
    assert.ok(window.memorySize() <= serie.memorySize())

    // A narrower window rebases onto the serie.
    const narrower = window.window(2, 1)
    assert.deepEqual([narrower.offset, narrower.length], [3, 1])
    assert.strictEqual(narrower.serie, serie)
    assert.equal(narrower.scalar(0).asJs(), 4)

    // The window as a serie is `slice`.
    assert.ok(window.intoSerie().equals(serie.slice(1, 3)))
    assert.equal(serie.window(6, 0).length, 0)
  }
  // A window has no public constructor: a serie hands it out.
  assert.throws(() => new WindowSerie(), /handed out by Serie/)
})

test('every edge is refused naming the serie and both counts', () => {
  const prices = column([1, 2, 3])
  assert.throws(() => prices.window(2, 2), /rows 2\.\.4 reach past the 3 rows price holds/)
  assert.throws(() => run([1]).window(0, 2), /reach past the 1 rows \$ holds/)
  const window = prices.window(1, 2)
  assert.throws(() => window.scalar(2), /row 2 is past the 2 rows price holds/)
  assert.throws(() => window.isNull(2))
  assert.throws(() => window.window(1, 2), /rows 1\.\.3 reach past the 2 rows price holds/)
  assert.throws(() => window.set(2, 0))
  assert.throws(() => window.swap(0, 2))
  assert.throws(
    () => window.splice(0, 1, []),
    /never grows or shrinks what it views: 0 rows cannot replace 1/,
  )
  assert.throws(() => window.splice(0, 3, [1, 2, 3]), /rows 0\.\.3 reach past the 2 rows price holds/)
  assert.throws(() => window.asTaken([0]), /1 indices cannot rearrange 2 rows/)
  // Nothing moved.
  assert.deepEqual(prices.asJs(), [1, 2, 3])

  // The window is read at each call: one the serie no longer reaches is
  // refused by the call that reads it, and answers again once it does.
  const shrinking = column([1, 2, 3, 4])
  const tail = shrinking.window(2, 2)
  shrinking.truncate(3)
  assert.throws(() => tail.scalar(0), /rows 2\.\.4 reach past the 3 rows price holds/)
  shrinking.push(9)
  assert.deepEqual(tail.asJs(), [3, 9])
})

test('identity is the window rows alone, like a serie', () => {
  const prices = column([9, 1, 2, 9])
  const other = run([1, 2])
  const window = prices.window(1, 2)
  const otherWindow = other.window(0, 2)
  assert.equal(window.equals(otherWindow), true)
  assert.equal(window.equals(other), true)
  assert.equal(window.equals(prices.window(2, 2)), false)
  assert.equal(window.toString(), 'price[Int64(Int64(1)), Int64(Int64(2))]')
  assert.equal(otherWindow.toString(), '$[Int64(Int64(1)), Int64(Int64(2))]')
  assert.throws(() => window.equals([1, 2]), /WindowSerie.equals takes a WindowSerie or a Serie/)
})

test('the reads of a window answer what the sliced serie answers', () => {
  const prices = column([9, 3, 1, 3, null, 0])
  const window = prices.window(1, 4)
  assert.equal(window.isSorted(), false)
  assert.equal(window.isUnique(), false)
  assert.equal(window.uniqueCount(), 3)
  assert.deepEqual(window.sortIndices().asJs(), [1, 0, 2, 3])
  assert.deepEqual(window.intoSorted().asJs(), [1, 3, 3, null])
  assert.deepEqual(window.intoSorted({ descending: true, nullsFirst: true }).asJs(), [null, 3, 3, 1])
  assert.deepEqual(window.intoUnique().asJs(), [3, 1, null])
  assert.deepEqual(window.intoReversed().asJs(), [null, 3, 1, 3])
  assert.deepEqual(window.intoTaken([1]).asJs(), [1])
  const mask = [true, false, false, true]
  assert.deepEqual(window.intoFiltered(mask).asJs(), [3, null])
  const groups = window.partitionBy(['a', 'b', 'a', 'b'])
  assert.equal(groups.length, 2)
  assert.equal(groups[0][0].asJs(), 'a')
  assert.deepEqual(groups[0][1].asJs(), [3, 3])
  // Every answer is a new serie under the serie's field, the serie as it was.
  assert.ok(window.intoSorted().field.equals(prices.field))
  assert.deepEqual(prices.asJs(), [9, 3, 1, 3, null, 0])

  // A record window hands its answers out as the leaf's class.
  const records = Serie.fromScalars(
    Field.from('quote: struct<venue: utf8 not null> not null'),
    [{ venue: 'XNYS' }, { venue: 'XNAS' }],
  )
  const recordWindow = records.window(0, 2)
  for (const answer of [
    recordWindow.intoSerie(),
    recordWindow.intoSorted(),
    recordWindow.intoUnique(),
    recordWindow.intoReversed(),
    recordWindow.intoTaken([0]),
    recordWindow.intoFiltered([true, false]),
    recordWindow.partitionBy([1, 1])[0][1],
  ]) {
    assert.ok(answer instanceof StructSerie)
  }
})

test('every write goes through the serie on the rebased range and never past the window', () => {
  const prices = column([1, 2, 3, 4, 5])
  const window = prices.window(1, 3)
  window.set(0, 20)
  window.swap(0, 2)
  assert.deepEqual(window.asJs(), [4, 3, 20])
  window.splice(1, 3, [30, 40])
  assert.deepEqual(prices.asJs(), [1, 4, 30, 40, 5])

  window.fill(7)
  assert.deepEqual(prices.asJs(), [1, 7, 7, 7, 5])
  // A value the field refuses refuses the write and leaves the column.
  assert.throws(() => window.fill(null))
  assert.throws(() => window.set(0, 'x'))
  assert.deepEqual(prices.asJs(), [1, 7, 7, 7, 5])

  const source = run([8, 9, 10])
  window.copyFrom(source.window(0, 3))
  assert.throws(() => window.copyFrom(source.window(0, 2)), /2 rows cannot replace 3/)
  assert.deepEqual(prices.asJs(), [1, 8, 9, 10, 5])
  // A whole serie copies as its one window.
  window.copyFrom(run([4, 5, 6]))
  assert.deepEqual(prices.asJs(), [1, 4, 5, 6, 5])
  // A window copied from the serie it views reads the rows as they were.
  prices.window(0, 2).copyFrom(prices.window(2, 2))
  assert.deepEqual(prices.asJs(), [5, 6, 5, 6, 5])
  assert.throws(() => window.copyFrom([1, 2, 3]), /WindowSerie.copyFrom takes a WindowSerie or a Serie/)

  // A run is written the same way.
  const values = run([1, 2, 3, 4, 5])
  const runWindow = values.window(1, 3)
  runWindow.set(1, 'mixed')
  runWindow.swap(0, 2)
  assert.deepEqual(values.asJs(), [1, 4, 'mixed', 2, 5])
})

test('the window writes sort, reverse and rearrange in place within the window', () => {
  // A primitive column sorts the window of its buffer where it stands,
  // absences gathered within the window.
  const prices = column([9, null, 3, 1, 2, 0])
  const window = prices.window(1, 4)
  assert.strictEqual(window.asSorted().asReversed(), window)
  assert.deepEqual(prices.asJs(), [9, null, 3, 2, 1, 0])
  window.asSorted({ nullsFirst: true })
  assert.deepEqual(prices.asJs(), [9, null, 1, 2, 3, 0])
  assert.strictEqual(prices.window(2, 3).asTaken([2, 0, 1]).serie, prices)
  assert.deepEqual(prices.asJs(), [9, null, 3, 1, 2, 0])

  // A string column writes the sorted rows back through splice.
  const venues = Serie.fromScalars(new Field('venue', 'utf8', false), ['z', 'c', 'a', 'b', 'y'])
  venues.window(1, 3).asSorted().asReversed()
  assert.deepEqual(venues.asJs(), ['z', 'c', 'b', 'a', 'y'])

  // A run sorts its values in place.
  const values = run([9, 3, 1, 2, 0])
  values.window(1, 3).asSorted({ descending: true })
  assert.deepEqual(values.asJs(), [9, 3, 2, 1, 0])
  values.window(0, 5).asReversed()
  assert.deepEqual(values.asJs(), [0, 1, 2, 3, 9])
})

test('a window over a shared column copies the column once and leaves the clone', () => {
  const prices = column([2, 1, 3])
  const other = prices.clone()
  other.window(0, 2).asSorted()
  assert.deepEqual(other.asJs(), [1, 2, 3])
  assert.deepEqual(prices.asJs(), [2, 1, 3])
})

test('the window natives stay outside the public surface', () => {
  for (const name of [
    '_asJsNative',
    '_iterNative',
    '_isSortedNative',
    '_sortIndicesNative',
    '_windowNative',
    '_windowByNative',
    '_intoSerieNative',
    '_intoSortedNative',
    '_intoUniqueNative',
    '_intoReversedNative',
    '_intoTakenNative',
    '_intoFilteredNative',
    '_partitionByNative',
    '_equalsNative',
    '_setNative',
    '_fillNative',
    '_copyFromNative',
    '_spliceNative',
    '_asSortedNative',
    '_asReversedNative',
    '_asTakenNative',
    '_sortIndicesByNative',
    '_intoSortByNative',
    '_asSortByNative',
  ]) {
    assert.equal(name in WindowSerie.prototype, false, name)
  }
  // A window never shrinks what it views, so it offers neither write that
  // would.
  assert.equal('asUnique' in WindowSerie.prototype, false)
  assert.equal('asFiltered' in WindowSerie.prototype, false)
})

test('a window sorts by key over its own rows alone', () => {
  const prices = column([9, 1, 3, 2])
  const window = prices.window(1, 3)
  assert.deepEqual(window.sortIndicesBy('price desc').asJs(), [1, 2, 0])
  assert.deepEqual(window.sortIndicesBy([{ term: 'price', descending: true }]).asJs(), [1, 2, 0])
  const sorted = window.intoSortBy('price desc')
  assert.ok(sorted instanceof Serie)
  assert.deepEqual(sorted.asJs(), [3, 2, 1])
  assert.deepEqual(prices.asJs(), [9, 1, 3, 2])
  // In place, written back over the window's range; every row outside it
  // untouched, and a refusal leaves the serie as it was.
  assert.strictEqual(window.asSortBy('price desc'), window)
  assert.deepEqual(prices.asJs(), [9, 3, 2, 1])
  assert.throws(() => window.asSortBy('tier'))
  assert.deepEqual(prices.asJs(), [9, 3, 2, 1])
  // A record window keys by its root's columns.
  const quotes = Serie.fromScalars(
    Field.from('quote: struct<venue: utf8 not null, price: int64 not null> not null'),
    [
      { venue: 'XNYS', price: 2 },
      { venue: 'XNAS', price: 1 },
      { venue: 'XNYS', price: 1 },
      { venue: 'AAAA', price: 0 },
    ],
  )
  const records = quotes.window(0, 3).intoSortBy('venue, price')
  assert.ok(records instanceof StructSerie)
  assert.deepEqual(records.asJs(), [
    { venue: 'XNAS', price: 1 },
    { venue: 'XNYS', price: 1 },
    { venue: 'XNYS', price: 2 },
  ])
  assert.deepEqual(records.declaredOrder(), ['venue', 'price'])
})

test('a window answers where its rows live through the serie it views', () => {
  const prices = Serie.fromScalars(
    new Field('price', 'int64', false),
    Array.from({ length: 1_024 }, (_, index) => index),
  )
  const window = prices.window(8, 64)
  assert.equal(window.isSpilled(), false)
  assert.equal(window.residentSize(), window.memorySize())
  prices.spill(new SpillOptions({ byteSize: 0 }))
  assert.equal(window.isSpilled(), true)
  assert.equal(window.residentSize(), 0)
  assert.equal(window.scalar(0).asJs(), 8)
  // A window over a run is resident.
  const values = run([1, 2, 3]).window(0, 2)
  assert.equal(values.isSpilled(), false)
  assert.equal(values.residentSize(), values.memorySize())
})

// ---------------------------------------------------------------------------
// windowBy: the windows of a window, and the record a window states
// ---------------------------------------------------------------------------

const MINUTE_NS = 60_000_000_000n

// Quotes `{venue, price}` whose root is nullable, so a row may be absent.
const quoteRows = (rows) =>
  Serie.fromScalars(
    new Field('quote', 'struct<venue: utf8, price: int64 not null>', true),
    rows.map((row) => (row === null ? null : { venue: row[0], price: row[1] })),
  )

const cuts = (windows) => windows.map(([key, window]) => [key.asJs(), window.offset, window.length])

// Every window's record as its natural JavaScript value.
const records = (windows) => windows.map(([, window]) => window.staticValues.asJs())

test('a window windowBy lent states its key, windownum and rownum', () => {
  const quotes = quoteRows([
    ['XNAS', 1],
    ['XNAS', 2],
    ['XNYS', 3],
    ['XNAS', 4],
  ])
  const windows = quotes.windowBy('venue')
  assert.deepEqual(records(windows), [
    { venue: 'XNAS', windownum: 0, rownum: 0 },
    { venue: 'XNYS', windownum: 1, rownum: 2 },
    { venue: 'XNAS', windownum: 2, rownum: 3 },
  ])
  // Two terms are two key cells, named as their projections are.
  const [[, first]] = quotes.windowBy('venue, price > 1 as late')
  assert.deepEqual(first.staticValues.asJs(), {
    venue: 'XNAS',
    late: false,
    windownum: 0,
    rownum: 0,
  })
})

test('a gathered window states a null rownum', () => {
  const quotes = quoteRows([['XNYS', 1], ['XNAS', 2], ['XNYS', 3], null, ['XNAS', 5]])
  const windows = quotes.windowBy('venue', true)
  // Gathered once in stable key order, an absent row's key last.
  assert.deepEqual(cuts(windows), [
    [['XNAS'], 0, 2],
    [['XNYS'], 2, 2],
    [null, 4, 1],
  ])
  const gathered = windows[0][1].serie
  assert.notStrictEqual(gathered, quotes)
  assert.ok(gathered instanceof StructSerie)
  assert.ok(windows.every(([, window]) => window.serie === gathered))
  assert.ok(gathered.equals(quotes.intoTaken([1, 4, 0, 2, 3])))
  assert.deepEqual(records(windows), [
    { venue: 'XNAS', windownum: 0, rownum: null },
    { venue: 'XNYS', windownum: 1, rownum: null },
    { venue: null, windownum: 2, rownum: null },
  ])
  // Over keys already in order nothing is gathered, and every rownum stands.
  const ordered = quoteRows([
    ['XNAS', 1],
    ['XNYS', 2],
  ]).windowBy('venue', true)
  assert.deepEqual(
    records(ordered).map((record) => record.rownum),
    [0, 1],
  )
})

test('a plain or narrowed window states none', () => {
  const quotes = quoteRows([
    ['XNAS', 1],
    ['XNAS', 2],
    ['XNYS', 3],
  ])
  assert.equal(quotes.window(0, 2).staticValues, null)
  const [[, xnas]] = quotes.windowBy('venue')
  assert.notEqual(xnas.staticValues, null)
  assert.equal(xnas.window(0, 1).staticValues, null)
  assert.equal(xnas.window(1, 1).staticValues, null)
  // The record is never the window's identity, and stays with the window.
  assert.ok(xnas.equals(quotes.window(0, 2)))
  assert.ok(xnas.intoSerie().equals(quotes.slice(0, 2)))
  assert.deepEqual(xnas.intoSerie().intoArrowBatch().schema.names, ['venue', 'price'])
})

test('the record is read through the generic scalar accessors', () => {
  const quotes = quoteRows([
    ['XNAS', 1],
    ['XNYS', 2],
  ])
  const [, [, xnys]] = quotes.windowBy('venue')
  const record = xnys.staticValues
  assert.ok(record instanceof Scalar)
  assert.equal(record.kind, 'struct')
  assert.equal(record.get('venue').asJs(), 'XNYS')
  assert.equal(record.get('windownum').asJs(), 1)
  assert.equal(record.path('.rownum').asJs(), 1)
  assert.equal(record.has('price'), false)
  assert.equal(record.get('price'), null)
  // Read again, it is the same value.
  assert.ok(xnys.staticValues.equals(record))
})

test('a window windows by its own rows over the serie', () => {
  const quotes = quoteRows([
    ['XNYS', 0],
    ['XNAS', 1],
    ['XNAS', 2],
    ['XNYS', 3],
    ['XNAS', 4],
  ])
  const tail = quotes.window(1, 4)
  const windows = tail.windowBy('venue')
  // Offsets are the serie's, every window over the serie object itself.
  assert.deepEqual(cuts(windows), [
    [['XNAS'], 1, 2],
    [['XNYS'], 3, 1],
    [['XNAS'], 4, 1],
  ])
  assert.ok(windows.every(([, window]) => window.serie === quotes))
  // `sorted` absent, `undefined` and `null` are its default, `false`.
  for (const spelled of [
    tail.windowBy('venue', false),
    tail.windowBy('venue', undefined),
    tail.windowBy('venue', null),
  ]) {
    assert.deepEqual(cuts(spelled), cuts(windows))
  }
  // A plain window states no record of its own, so its windows number their
  // rows from its first.
  assert.deepEqual(
    records(windows).map((record) => record.rownum),
    [0, 2, 3],
  )
  // Sorted over keys out of order, the gather takes this window's rows alone.
  const gathered = tail.windowBy('venue', true)
  assert.deepEqual(cuts(gathered), [
    [['XNAS'], 0, 3],
    [['XNYS'], 3, 1],
  ])
  assert.ok(gathered[0][1].serie.equals(quotes.intoTaken([1, 2, 4, 3])))
  assert.throws(() => tail.windowBy('venue', 'yes'), {
    name: 'TypeError',
    message: /WindowSerie\.windowBy sorted must be a boolean, got string/,
  })
  assert.throws(() => tail.windowBy('venue,'), /expected a value or a name/)
})

test('a window of a lent window keeps the outer cells and an absolute rownum', () => {
  // Halves of an hour: XNAS XNAS XNYS in the first, XNAS XNYS XNYS in the
  // second.
  const root = new Field('quote', 'struct<venue: utf8, price: int64 not null, ts: timestamp(ns, UTC)>', true)
  const minutes = [0, 14, 15, 31, 40, 45]
  const venues = ['XNAS', 'XNAS', 'XNYS', 'XNAS', 'XNYS', 'XNYS']
  const quotes = Serie.fromScalars(
    root,
    venues.map((venue, index) => ({ venue, price: index, ts: BigInt(minutes[index]) * MINUTE_NS })),
  )
  const halves = quotes.windowBy('minutes(ts, 30) as half')
  const [, second] = halves[1]
  const inner = second.windowBy('venue')
  // Over the serie object itself, at the serie's offsets.
  assert.deepEqual(cuts(inner), [
    [['XNAS'], 3, 1],
    [['XNYS'], 4, 2],
  ])
  assert.ok(inner.every(([, window]) => window.serie === quotes))
  const expected = [
    { half: 1, venue: 'XNAS', windownum: 0, rownum: 3 },
    { half: 1, venue: 'XNYS', windownum: 1, rownum: 4 },
  ]
  assert.deepEqual(records(inner), expected)
  // The windows of the second window of the stream state the same records.
  const walk = SerieReader.fromSerie(quotes).windowBy('minutes(ts, 30) as half')
  walk.next()
  const streamed = [...walk.next().value.windowBy('venue')]
  assert.deepEqual(
    streamed.map((window) => window.staticValues.asJs()),
    expected,
  )
  // A third level keeps both outer cells, and its rownum stays absolute.
  const [, xnys] = inner[1]
  assert.deepEqual(records(xnys.windowBy('price')), [
    { half: 1, venue: 'XNYS', price: 4, windownum: 0, rownum: 4 },
    { half: 1, venue: 'XNYS', price: 5, windownum: 1, rownum: 5 },
  ])
  // A gather under a lent window states no rownum, over one new serie.
  const gathered = halves[0][1].windowBy('venue', true)
  assert.deepEqual(cuts(gathered), [
    [['XNAS'], 0, 2],
    [['XNYS'], 2, 1],
  ])
  assert.ok(gathered.every(([, window]) => window.serie === halves[0][1].serie))
  const outOfOrder = second.windowBy('price < 4 as early', true)
  assert.notStrictEqual(outOfOrder[0][1].serie, quotes)
  assert.deepEqual(records(outOfOrder), [
    { half: 1, early: false, windownum: 0, rownum: null },
    { half: 1, early: true, windownum: 1, rownum: null },
  ])
  // A narrower window of a lent window is the rows alone again.
  assert.deepEqual(records(second.window(1, 2).windowBy('venue')), [
    { venue: 'XNYS', windownum: 0, rownum: 0 },
  ])
  // A key cell folding onto a kept cell is refused naming both.
  assert.throws(
    () => second.windowBy('venue as HALF'),
    /the key cell "HALF" collides with the static value "half"/,
  )
})

test('a lent window keeps the record of the rows it was cut from', () => {
  const quotes = quoteRows([
    ['XNAS', 1],
    ['XNYS', 2],
  ])
  const [[, xnas]] = quotes.windowBy('venue')
  // A write on the serie reaches the window's rows, and its record stays
  // what the windowing read.
  quotes.set(0, { venue: 'XLON', price: 9 })
  assert.deepEqual(xnas.asJs(), [{ venue: 'XLON', price: 9 }])
  assert.equal(xnas.staticValues.get('venue').asJs(), 'XNAS')
  assert.equal(quotes.windowBy('venue')[0][1].staticValues.get('venue').asJs(), 'XLON')
})
