'use strict'

// Pins node/src/chunked_serie.rs: many columns under one field, held apart -
// an Arrow JS vector of several Data, and a table of one batch per chunk -
// every verb redirected into the core, and Arrow crossing one chunk per batch.

const assert = require('node:assert/strict')
const test = require('node:test')

const arrow = require('apache-arrow')
const binding = require('yggdryl')
const { BatchReader, ChunkedSerie, DataType, Field, Scalar, Serie, StructSerie, fields } = binding

const trades = () =>
  fields.struct('row', [Field.from('id: int64'), Field.from('symbol: utf8')], {
    nullable: false,
  })

function narrow(ids, symbols) {
  return new arrow.Table({
    id: arrow.vectorFromArray(ids, new arrow.Int32()),
    symbol: arrow.vectorFromArray(symbols, new arrow.Utf8()),
  })
}

// One table of two batches: two rows, then one.
function twoBatches() {
  const table = new arrow.Table([
    ...narrow([1, 2], ['AAPL', 'MSFT']).batches,
    ...narrow([3], ['NVDA']).batches,
  ])
  assert.equal(table.batches.length, 2)
  return table
}

// One vector of two Data: two rows, then two with a null.
function twoData() {
  const vector = arrow
    .vectorFromArray([1, 2], new arrow.Int32())
    .concat(arrow.vectorFromArray([3, null], new arrow.Int32()))
  assert.equal(vector.data.length, 2)
  return vector
}

const ids = (rows) => Serie.fromScalars(fields.int64('id'), rows)

test('the private natives stay outside the public surface', () => {
  for (const name of [
    '_emptyNative',
    '_fromSerieNative',
    '_fromSeriesNative',
    '_fromArrowArrayIpcNative',
    '_fromArrowBatchIpcNative',
    '_fromArrowReaderNative',
  ]) {
    assert.equal(Object.hasOwn(ChunkedSerie, name), false, name)
  }
  for (const name of [
    '_chunksNative',
    '_chunkNative',
    '_asJsNative',
    '_iterNative',
    '_pushChunkNative',
    '_intoSerieNative',
    '_castNative',
    '_intoArrowArrayIpcNative',
    '_equalsNative',
    '_compareNative',
  ]) {
    assert.equal(name in ChunkedSerie.prototype, false, name)
  }
  // No constructor: a chunked serie is built through a static.
  assert.throws(() => new ChunkedSerie(), /no `constructor`/)
  assert.equal(ChunkedSerie.name, 'ChunkedSerie')
})

test('an Arrow JS vector of two Data lands as two chunks and comes back as two', () => {
  const vector = twoData()
  const chunked = ChunkedSerie.fromArrowArray(vector)
  assert.ok(chunked instanceof ChunkedSerie)
  assert.equal(chunked.numChunks, 2)
  assert.equal(chunked.length, 4)
  assert.equal(chunked.field.name, 'item')
  assert.equal(chunked.field.nullable, true)
  assert.equal(chunked.field.dtype.toString(), 'int32')
  assert.equal(chunked.dtype.toString(), DataType.from('serie<item: int32>').toString())
  assert.deepEqual(chunked.asJs(), [1, 2, 3, null])
  assert.deepEqual(
    chunked.chunks.map((chunk) => chunk.asJs()),
    [[1, 2], [3, null]],
  )
  assert.ok(chunked.chunks.every((chunk) => chunk instanceof Serie))

  const back = chunked.intoArrowArray()
  assert.ok(arrow.isArrowVector(back))
  assert.equal(back.data.length, chunked.numChunks)
  assert.deepEqual(
    back.data.map((data) => data.length),
    [2, 2],
  )
  assert.deepEqual([...back], [1, 2, 3, null])
  assert.equal(back.type.toString(), 'Int32')

  // A field casts every chunk by one plan; an expression is a field.
  const wide = ChunkedSerie.fromArrowArray(vector, fields.int64('id'))
  assert.equal(wide.numChunks, 2)
  assert.ok(wide.field.equals(fields.int64('id')))
  assert.deepEqual(wide.asJs(), [1, 2, 3, null])
  assert.equal(wide.intoArrowArray().type.toString(), 'Int64')
  assert.ok(ChunkedSerie.fromArrowArray(vector, 'id: int64').field.equals(fields.int64('id')))

  // A vector of one Data is one chunk, nullable only where a row is null.
  const one = ChunkedSerie.fromArrowArray(arrow.vectorFromArray([1, 2], new arrow.Int32()))
  assert.equal(one.numChunks, 1)
  assert.equal(one.field.nullable, false)
  assert.throws(() => ChunkedSerie.fromArrowArray([1, 2]), /Apache Arrow Vector/)
})

test('the three cast answers reach the core', () => {
  const overflowing = arrow
    .vectorFromArray([7, 130], new arrow.Int32())
    .concat(arrow.vectorFromArray([null], new arrow.Int32()))
  const required = fields.int8('quantity', { nullable: false })

  // Safe by default: the value the target cannot hold is null, and the
  // required column repairs every absent row with its default.
  const repaired = ChunkedSerie.fromArrowArray(overflowing, required)
  assert.equal(repaired.numChunks, 2)
  assert.deepEqual(repaired.asJs(), [7, 0, 0])

  assert.throws(
    () => ChunkedSerie.fromArrowArray(overflowing, required, { nullability: 'strict' }),
    /required Arrow field \$\.quantity holds 1 null values/,
  )
  assert.throws(
    () => ChunkedSerie.fromArrowArray(overflowing, required, { safe: false }),
    /Can't cast value 130 to type Int8/,
  )
  assert.throws(
    () => ChunkedSerie.fromArrowArray(overflowing, required, { nullability: 'lenient' }),
    /expected one of default, strict/,
  )
  assert.throws(
    () => ChunkedSerie.fromArrowArray(overflowing, required, { nullable: 'strict' }),
    /cast options take safe, nullability and representation/,
  )
  // An absent answer is skipped.
  assert.deepEqual(
    ChunkedSerie.fromArrowArray(overflowing, required, {
      safe: undefined,
      nullability: undefined,
      representation: undefined,
    }).asJs(),
    [7, 0, 0],
  )
  const unsigned = arrow.vectorFromArray([2 ** 32 - 1], new arrow.Uint32())
  assert.deepEqual(
    ChunkedSerie.fromArrowArray(unsigned, fields.int32('digest'), {
      representation: 'bits',
    }).asJs(),
    [-1],
  )
})

test('an Arrow JS table of two batches lands as one record chunk per batch', () => {
  const table = twoBatches()
  const chunked = ChunkedSerie.fromArrowBatch(table)
  assert.equal(chunked.numChunks, 2)
  assert.equal(chunked.length, 3)
  assert.equal(chunked.field.name, 'row')
  assert.equal(chunked.field.nullable, false)
  assert.ok(chunked.chunks.every((chunk) => chunk instanceof StructSerie))
  assert.deepEqual(chunked.asJs(), [
    { id: 1, symbol: 'AAPL' },
    { id: 2, symbol: 'MSFT' },
    { id: 3, symbol: 'NVDA' },
  ])

  // A table's column is the child of every batch, kept apart.
  const symbols = chunked.child('symbol')
  assert.ok(symbols instanceof ChunkedSerie)
  assert.equal(symbols.numChunks, 2)
  assert.deepEqual(symbols.asJs(), ['AAPL', 'MSFT', 'NVDA'])

  // Back out, one batch per chunk, holding the same rows.
  const out = chunked.intoArrowTable()
  assert.equal(out.batches.length, 2)
  assert.deepEqual(
    out.batches.map((batch) => batch.numRows),
    table.batches.map((batch) => batch.numRows),
  )
  assert.deepEqual([...out.getChild('symbol')], [...table.getChild('symbol')])
  assert.deepEqual([...out.getChild('id')], [1, 2, 3])

  // Cast into a root by one plan, every batch.
  const cast = ChunkedSerie.fromArrowBatch(table, trades())
  assert.equal(cast.numChunks, 2)
  assert.ok(cast.field.equals(trades()))
  assert.deepEqual([...cast.intoArrowTable().getChild('id')], [1n, 2n, 3n])

  // One batch is one chunk, and a table of no batch is no chunk.
  assert.equal(ChunkedSerie.fromArrowBatch(table.batches[0]).numChunks, 1)
  const none = ChunkedSerie.fromArrowBatch(new arrow.Table(table.schema))
  assert.equal(none.numChunks, 0)
  assert.equal(none.isEmpty(), true)
  assert.equal(none.child('symbol').field.name, 'symbol')
  assert.throws(() => ChunkedSerie.fromArrowBatch([], trades()), TypeError)
})

test('a native BatchReader is consumed, one chunk per batch', () => {
  const stream = BatchReader.from(twoBatches())
  const chunked = ChunkedSerie.fromArrowReader(stream, trades())
  assert.equal(stream.consumed, true)
  assert.equal(chunked.numChunks, 2)
  assert.ok(chunked.field.equals(trades()))
  assert.deepEqual(chunked.child('id').asJs(), [1, 2, 3])

  const own = ChunkedSerie.fromArrowReader(BatchReader.from(twoBatches()))
  assert.equal(own.field.name, 'row')
  assert.equal(own.child('id').field.dtype.toString(), 'int32')

  assert.throws(() => ChunkedSerie.fromArrowReader(stream, trades()), /already been consumed/)
  assert.throws(
    () => ChunkedSerie.fromArrowReader(twoBatches()),
    /ChunkedSerie\.fromArrowReader takes a native BatchReader/,
  )
})

test('held columns are chunks under one field', () => {
  const first = ids([1n, 2n])
  const one = ChunkedSerie.fromSerie(first)
  assert.equal(one.numChunks, 1)
  assert.ok(one.field.equals(first.field))
  assert.ok(one.equals(first))

  // With no field, the first chunk's, nullable where any chunk's is.
  const required = Serie.fromScalars(fields.int64('id', { nullable: false }), [1n])
  const chunked = ChunkedSerie.fromSeries([required, ids([2n, null])])
  assert.equal(chunked.numChunks, 2)
  assert.equal(chunked.field.nullable, true)
  assert.deepEqual(chunked.asJs(), [1, 2, null])

  // With no field, another datatype is refused naming the chunk; a declared
  // field casts any other layout into it, and a set is an iterable of chunks.
  const narrowIds = Serie.fromScalars(fields.int32('id'), [3])
  assert.throws(
    () => ChunkedSerie.fromSeries([first, narrowIds]),
    /chunk 1 of "id" is int32, and the first chunk int64/,
  )
  const wide = ChunkedSerie.fromSeries(new Set([first, narrowIds]), 'id: int64', {
    safe: false,
  })
  assert.equal(wide.numChunks, 2)
  assert.deepEqual(wide.asJs(), [1, 2, 3])
  assert.ok(wide.chunks.every((chunk) => chunk.field.equals(fields.int64('id'))))

  // No chunk needs a field; the empty chunked serie of a field is no chunk.
  assert.throws(() => ChunkedSerie.fromSeries([]), /a chunked serie of no chunk names no field/)
  const empty = ChunkedSerie.fromSeries([], fields.int64('id'))
  assert.equal(empty.numChunks, 0)
  assert.equal(empty.length, 0)
  const declared = ChunkedSerie.empty('id: int64')
  assert.equal(declared.numChunks, 0)
  assert.ok(declared.field.equals(fields.int64('id')))
  assert.equal(declared.intoSerie().length, 0)
  assert.ok(declared.equals(empty))

  // A run names no field, and a chunk is a Serie, never a converted value.
  const run = new Serie([1, 2])
  assert.throws(() => ChunkedSerie.fromSerie(run), /a schema-free run declares no field/)
  assert.throws(() => ChunkedSerie.fromSeries([first, run]), /a schema-free run declares no field/)
  assert.throws(() => ChunkedSerie.fromSerie([1n]), /ChunkedSerie\.fromSerie takes a Serie/)
  assert.throws(() => ChunkedSerie.fromSeries([[1n]]), /ChunkedSerie\.fromSeries takes a Serie/)
  assert.throws(() => ChunkedSerie.fromSeries(7), /an iterable of Serie/)
})

test('rows are read across chunk boundaries', () => {
  const chunked = ChunkedSerie.fromSeries([ids([1n, 2n]), ids([3n, null])])
  assert.equal(chunked.nullCount(), 1)
  assert.equal(chunked.isEmpty(), false)
  assert.equal(chunked.isNull(3), true)
  assert.equal(chunked.isNull(2), false)
  assert.ok(chunked.scalar(2).equals(Scalar.from(3n)))
  assert.ok(chunked.at(1).equals(Scalar.from(2n)))
  assert.equal(chunked.at(4), null)
  assert.deepEqual(
    chunked.rows().map((row) => row.asJs()),
    [1, 2, 3, null],
  )
  assert.deepEqual(
    [...chunked].map((row) => row.asJs()),
    [1, 2, 3, null],
  )
  assert.ok([...chunked].every((row) => row instanceof Scalar))
  assert.deepEqual(chunked.asJs({ maxDepth: 4 }), [1, 2, 3, null])
  assert.equal(JSON.stringify(ChunkedSerie.fromArrowArray(twoData())), '[1,2,3,null]')
  // Rendered as the column of its rows would be.
  assert.equal(chunked.toString(), chunked.intoSerie().toString())

  assert.throws(() => chunked.scalar(4), /row 4 is past the end of 4 rows/)
  assert.throws(() => chunked.isNull(9), /row 9 is past the end of 4 rows/)
  assert.throws(() => chunked.scalar(-1), /index must be a non-negative whole number/)
  assert.throws(() => chunked.chunk(0.5), /index must be a non-negative whole number/)
  assert.throws(() => chunked.asJs({ maxDepth: 0 }))
})

test('a window keeps the chunks it reaches, nothing copied', () => {
  const chunked = ChunkedSerie.fromSeries([ids([1n, 2n]), ids([3n, 4n]), ids([5n])])
  const middle = chunked.slice(1, 3)
  assert.equal(middle.numChunks, 2)
  assert.deepEqual(middle.asJs(), [2, 3, 4])
  assert.equal(chunked.slice(0, 2).numChunks, 1)
  assert.equal(chunked.slice(2, 0).numChunks, 0)
  assert.equal(chunked.slice(0, 5).numChunks, 3)
  assert.throws(() => chunked.slice(4, 2), /rows 4\.\.6 reach past the 5 rows id holds/)

  // A chunk is handed out by position, and none past the last.
  assert.deepEqual(chunked.chunk(1).asJs(), [3, 4])
  assert.equal(chunked.chunk(3), null)
})

test('a record child is the child of every chunk', () => {
  const chunked = ChunkedSerie.fromArrowBatch(twoBatches(), trades())
  const byIndex = chunked.childAt(1)
  assert.ok(byIndex.equals(chunked.child('symbol')))
  assert.equal(chunked.childAt(2), null)
  assert.equal(chunked.child('venue'), null)
  assert.deepEqual(
    chunked.children().map((child) => [child.field.name, child.numChunks]),
    [
      ['id', 2],
      ['symbol', 2],
    ],
  )
  assert.deepEqual(chunked.getChildByPath('symbol').asJs(), ['AAPL', 'MSFT', 'NVDA'])
  assert.equal(chunked.getChildByPath('venue'), null)
  assert.throws(() => chunked.getChildByPath('[['), /invalid field path expression/)
  assert.equal(chunked.items(), null)

  // A sequence's items are the items of every chunk.
  const lists = fields.serie('values', fields.int64('item'))
  const nested = ChunkedSerie.fromSeries([
    Serie.fromScalars(lists, [[1n, 2n]]),
    Serie.fromScalars(lists, [[3n]]),
  ])
  const items = nested.items()
  assert.equal(items.numChunks, 2)
  assert.deepEqual(items.asJs(), [1, 2, 3])
  assert.deepEqual(nested.children(), [])

  // An empty chunked serie still answers its children, from the field.
  const empty = ChunkedSerie.empty(trades())
  assert.deepEqual(
    empty.children().map((child) => [child.field.name, child.numChunks]),
    [
      ['id', 0],
      ['symbol', 0],
    ],
  )
})

test('pushChunk appends as it stands or cast, and a refusal changes nothing', () => {
  const chunked = ChunkedSerie.fromSerie(
    Serie.fromScalars(fields.int64('id', { nullable: false }), [1n]),
  )
  chunked.pushChunk(Serie.fromScalars(fields.int64('id', { nullable: false }), [2n]))
  chunked.pushChunk(Serie.fromScalars(fields.int32('id'), [3]))
  assert.equal(chunked.numChunks, 3)
  assert.deepEqual(chunked.asJs(), [1, 2, 3])

  const absent = Serie.fromScalars(fields.int32('id'), [null])
  assert.throws(
    () => chunked.pushChunk(absent, { nullability: 'strict' }),
    /required Arrow field \$\.id holds 1 null values/,
  )
  assert.throws(() => chunked.pushChunk(new Serie([4n])), /a schema-free run declares no field/)
  assert.throws(() => chunked.pushChunk([4n]), /ChunkedSerie\.pushChunk takes a Serie/)
  assert.throws(
    () => chunked.pushChunk(absent, { strict: true }),
    /cast options take safe, nullability and representation/,
  )
  assert.equal(chunked.numChunks, 3)
  assert.equal(chunked.length, 3)

  // Under the default policy the required column repairs the absent row.
  chunked.pushChunk(absent)
  assert.deepEqual(chunked.asJs(), [1, 2, 3, 0])
})

test('intoSerie joins once and hands the column out as its leaf class', () => {
  const chunked = ChunkedSerie.fromArrowBatch(twoBatches(), trades())
  const joined = chunked.intoSerie()
  assert.ok(joined instanceof StructSerie)
  assert.equal(joined.length, 3)
  assert.ok(joined.field.equals(trades()))
  assert.ok(chunked.equals(joined))
  assert.deepEqual(joined.names, ['id', 'symbol'])
  assert.ok(chunked.chunk(0) instanceof StructSerie)
})

test('a cast is one plan applied to every chunk', () => {
  const chunked = ChunkedSerie.fromSeries([
    Serie.fromScalars(fields.int32('id'), [1, 2]),
    Serie.fromScalars(fields.int32('id'), [null]),
  ])
  const wide = chunked.cast(fields.int64('id'))
  assert.ok(wide instanceof ChunkedSerie)
  assert.equal(wide.numChunks, 2)
  assert.deepEqual(wide.asJs(), [1, 2, null])
  assert.ok(chunked.cast(chunked.field).equals(chunked))
  assert.ok(chunked.cast('id: int64').field.equals(fields.int64('id')))

  // A DataType is its required `value` field.
  const typed = chunked.cast(DataType.from('int64'))
  assert.equal(typed.field.name, 'value')
  assert.equal(typed.field.nullable, false)
  assert.deepEqual(typed.asJs(), [1, 2, 0])
  assert.throws(
    () => chunked.cast(fields.int64('id', { nullable: false }), { nullability: 'strict' }),
    /required Arrow field \$\.id holds 1 null values/,
  )
})

test('equality and order are the rows alone, however they are cut', () => {
  const cut = ChunkedSerie.fromSeries([ids([1n]), ids([2n, 3n])])
  const whole = ChunkedSerie.fromSerie(ids([1n, 2n, 3n]))
  assert.ok(cut.equals(whole))
  assert.ok(cut.equals(ids([1n, 2n, 3n])))
  assert.equal(cut.equals(ids([1n, 2n])), false)
  assert.equal(cut.compare(whole), 0)
  assert.equal(cut.compare(ids([1n, 2n, 3n])), 0)
  assert.equal(cut.compare(ChunkedSerie.fromSerie(ids([1n, 3n]))), -1)
  assert.equal(cut.compare(ids([1n, 2n])), 1)
  assert.throws(() => cut.equals([1n, 2n, 3n]), /ChunkedSerie\.equals takes a ChunkedSerie or a Serie/)
  assert.throws(() => cut.compare(null), /ChunkedSerie\.compare takes a ChunkedSerie or a Serie/)

  // A clone shares the chunks, and appending to it leaves the original alone.
  const copy = cut.clone()
  assert.ok(copy instanceof ChunkedSerie)
  assert.ok(copy.equals(cut))
  copy.pushChunk(ids([4n]))
  assert.equal(copy.numChunks, 3)
  assert.equal(cut.numChunks, 2)
})

test('a reader and a table cross one batch per chunk, and an absent record row is refused', () => {
  // A leaf column's chunks are each the one column of a `row` root.
  const chunked = ChunkedSerie.fromArrowArray(twoData(), fields.int64('id'))
  const reader = chunked.intoArrowReader()
  assert.ok(reader instanceof BatchReader)
  assert.equal(reader.field.name, 'row')
  assert.deepEqual(
    Array.from(reader.field.dtype, (child) => child.name),
    ['id'],
  )
  const table = chunked.intoArrowTable()
  assert.equal(table.batches.length, 2)
  assert.deepEqual([...table.getChild('id')], [1n, 2n, 3n, null])

  // A record's chunks are the batches they are, under the record's name.
  const named = fields.struct('trades', [Field.from('id: int64')], { nullable: false })
  const records = ChunkedSerie.fromSerie(Serie.fromScalars(named, [{ id: 1n }]))
  assert.equal(records.intoArrowReader().field.name, 'trades')

  // A table cannot state an absent row; the array does.
  const nullable = Field.from('row: struct<id: int64>')
  const absent = ChunkedSerie.fromSerie(Serie.fromScalars(nullable, [{ id: 1n }, null]))
  for (const cross of [() => absent.intoArrowTable(), () => absent.intoArrowReader()]) {
    assert.throws(cross, /record column "row" holds 1 absent rows, which a table cannot state/)
  }
  const vector = absent.intoArrowArray()
  assert.equal(vector.length, 2)
  assert.equal(vector.data.length, 1)
  assert.equal(vector.get(1), null)

  // No chunk is a vector and a table of no row, typed by the field. Arrow
  // JS holds neither without a Data or a batch: the vector's one Data is
  // empty and comes back as one empty chunk, while the table's one batch is
  // Arrow JS's placeholder, which it never writes, so it comes back as none.
  const empty = ChunkedSerie.empty(trades())
  const emptyVector = empty.intoArrowArray()
  assert.equal(emptyVector.length, 0)
  assert.equal(emptyVector.type.toString(), 'Struct<{id:Int64, symbol:Utf8}>')
  assert.equal(emptyVector.data.length, 1)
  assert.equal(ChunkedSerie.fromArrowArray(emptyVector).numChunks, 1)
  const emptyTable = empty.intoArrowTable()
  assert.equal(emptyTable.numRows, 0)
  assert.equal(ChunkedSerie.fromArrowBatch(emptyTable).numChunks, 0)
})

test('a vector crosses as one chunk per Data, an empty one included', () => {
  const vector = arrow
    .vectorFromArray([1n, 2n], new arrow.Int64())
    .concat(arrow.vectorFromArray([], new arrow.Int64()))
    .concat(arrow.vectorFromArray([3n], new arrow.Int64()))
  assert.deepEqual(
    vector.data.map((data) => data.length),
    [2, 0, 1],
  )
  const chunked = ChunkedSerie.fromArrowArray(vector)
  assert.equal(chunked.numChunks, 3)
  assert.deepEqual(
    chunked.chunks.map((chunk) => chunk.length),
    [2, 0, 1],
  )
  assert.deepEqual(chunked.asJs(), [1, 2, 3])
})
