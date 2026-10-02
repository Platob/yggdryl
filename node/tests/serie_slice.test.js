'use strict'

// Pins node/src/serie_slice.rs: a window over a serie that reads and writes
// through the serie at each call, every index window-relative. Each case
// mirrors rust/tests/root/serie_slice.rs by name.

const assert = require('node:assert/strict')
const test = require('node:test')

const { Field, Serie, SerieSlice, StructSerie } = require('yggdryl')

// Prices as an int64 column, `null` an absent row.
const column = (values) =>
  Serie.fromScalars(new Field('price', 'int64', values.includes(null)), values)
const run = (values) => new Serie(values)

test('a window reads through the serie, window-relative, on both leaves', () => {
  for (const serie of [column([1, 2, null, 4, 5, 6]), run([1, 2, null, 4, 5, 6])]) {
    const window = serie.window(1, 3)
    assert.ok(window instanceof SerieSlice)
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
  assert.throws(() => new SerieSlice(), /handed out by Serie/)
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
  assert.throws(() => window.equals([1, 2]), /SerieSlice.equals takes a SerieSlice or a Serie/)
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
  assert.throws(() => window.copyFrom([1, 2, 3]), /SerieSlice.copyFrom takes a SerieSlice or a Serie/)

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
  ]) {
    assert.equal(name in SerieSlice.prototype, false, name)
  }
  // A window never shrinks what it views, so it offers neither write that
  // would.
  assert.equal('asUnique' in SerieSlice.prototype, false)
  assert.equal('asFiltered' in SerieSlice.prototype, false)
})
