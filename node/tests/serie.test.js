'use strict'

// Pins node/src/serie.rs: the Serie Arrow doors, the one cast a column takes,
// and the StreamChunkedSerie stream, each redirected into the core with the three
// cast answers the caller gave.

const assert = require('node:assert/strict')
const test = require('node:test')

const arrow = require('apache-arrow')
const binding = require('yggdryl')
const {
  BatchReader,
  ChunkedSerie,
  DataType,
  Field,
  Selector,
  Serie,
  StreamChunkedSerie,
  StreamKeySerie,
  SpillOptions,
  StructSerie,
  Term,
  WindowSerie,
  fields,
} = binding

const trades = () =>
  fields.struct('row', [Field.from('id: int64'), Field.from('symbol: utf8')], {
    nullable: false,
  })

// The trades root with its `id` declared as `dtype`.
const tradesWith = (dtype) =>
  fields.struct('row', [Field.from(`id: ${dtype}`), Field.from('symbol: utf8')], {
    nullable: false,
  })

function narrow(ids, symbols) {
  return new arrow.Table({
    id: arrow.vectorFromArray(ids, new arrow.Int32()),
    symbol: arrow.vectorFromArray(symbols, new arrow.Utf8()),
  })
}

test('the private Arrow bridges stay outside the public surface', () => {
  for (const name of [
    '_fromDefaultNative',
    '_fromArrowArrayIpcNative',
    '_fromArrowBatchIpcNative',
    '_fromArrowReaderNative',
  ]) {
    assert.equal(Object.hasOwn(Serie, name), false, name)
  }
  for (const name of ['_castNative', '_intoArrowScalarIpcNative']) {
    assert.equal(name in Serie.prototype, false, name)
  }
  assert.equal(Object.hasOwn(StreamChunkedSerie, '_fromArrowReaderNative'), false)
  assert.equal(Object.hasOwn(StreamChunkedSerie, '_fromSerieNative'), false)
  assert.equal(Object.hasOwn(StreamChunkedSerie, '_fromChunkedNative'), false)
  assert.equal('_nextNative' in StreamChunkedSerie.prototype, false)
  assert.equal('_castNative' in StreamChunkedSerie.prototype, false)
  // The retired Field and DataType casts have no alias.
  for (const name of [
    'cast',
    'castArrow',
    'castArrowArray',
    'castArrowBatch',
    'castArrowReader',
    'defaultArrowScalar',
  ]) {
    assert.equal(name in Field.prototype, false, name)
  }
  assert.equal('defaultArrowScalar' in DataType.prototype, false)
  assert.equal('fromArrowTable' in Serie, false)
})

test('a serie layout is handed out as its leaf class, with the verbs it lends', () => {
  const item = fields.int64('item', { nullable: false })
  const rows = [[1, 2], [3, 4]]
  for (const [factory, leaf, verbs] of [
    [fields.serie('values', item), binding.SerieSerie, ['offsets']],
    [fields.largeSerie('values', item), binding.LargeSerieSerie, ['offsets']],
    [fields.serieView('values', item), binding.SerieViewSerie, ['offsets', 'sizes']],
    [fields.largeSerieView('values', item), binding.LargeSerieViewSerie, ['offsets', 'sizes']],
    [fields.fixedSizeSerie('values', item, 2), binding.FixedSizeSerieSerie, ['width']],
  ]) {
    const serie = Serie.fromScalars(factory, rows)
    assert.ok(serie instanceof leaf, leaf.name)
    assert.ok(serie instanceof Serie, leaf.name)
    assert.equal(serie.field.dtype.id, factory.dtype.id, leaf.name)
    assert.deepEqual(serie.asJs(), rows, leaf.name)
    for (const verb of verbs) assert.notEqual(serie[verb], undefined, `${leaf.name}.${verb}`)
    assert.deepEqual(serie.row(1).asJs(), [3, 4], leaf.name)
    assert.throws(() => new leaf(), /handed out by Serie/)
  }
  assert.equal(Serie.fromScalars(fields.fixedSizeSerie('values', item, 2), rows).width, 2)
  // The leaf classes carry the layout's own name; the list names are retired.
  for (const name of [
    'ListSerie',
    'LargeListSerie',
    'ListViewSerie',
    'LargeListViewSerie',
    'FixedSizeListSerie',
  ]) {
    assert.equal(name in binding, false, name)
  }
})

test('an int32 vector lands under an int64 field, and its own field without one', () => {
  const vector = arrow.vectorFromArray([1, 2, null], new arrow.Int32())

  const own = Serie.fromArrowArray(vector)
  assert.equal(own.field.name, 'item')
  assert.equal(own.field.nullable, true)
  assert.equal(own.field.dtype.toString(), 'int32')
  assert.deepEqual(own.asJs(), [1, 2, null])
  const present = Serie.fromArrowArray(arrow.vectorFromArray([1, 2], new arrow.Int32()))
  assert.equal(present.field.name, 'item')
  assert.equal(present.field.nullable, false)

  const wide = Serie.fromArrowArray(vector, fields.int64('id'))
  assert.equal(wide.field.name, 'id')
  assert.equal(wide.field.dtype.toString(), 'int64')
  assert.deepEqual(wide.asJs(), [1, 2, null])
  assert.equal(wide.intoArrowArray().type.toString(), 'Int64')

  // A field expression is a field.
  assert.ok(Serie.fromArrowArray(vector, 'id: int64').field.equals(fields.int64('id')))
  assert.throws(() => Serie.fromArrowArray([1, 2], fields.int64('id')), /Apache Arrow Vector/)
})

test('a vector crossing as several batches is cast once, as one column', () => {
  const chunked = arrow.vectorFromArray([1, 2], new arrow.Int32()).concat(
    arrow.vectorFromArray([3], new arrow.Int32()),
  )
  assert.equal(chunked.data.length, 2)
  const serie = Serie.fromArrowArray(chunked, fields.int64('id'))
  assert.equal(serie.length, 3)
  assert.deepEqual(serie.asJs(), [1, 2, 3])
})

test('the two cast answers reach the core; absence is the target field own', () => {
  const overflowing = arrow.vectorFromArray([7, 130, null], new arrow.Int32())
  const required = fields.int8('quantity', { nullable: false })

  // A required column refuses a value it cannot convert, by that value,
  // whatever `safe` says.
  assert.throws(
    () => Serie.fromArrowArray(overflowing, required),
    /Can't cast value 130 to type Int8/,
  )
  assert.throws(
    () => Serie.fromArrowArray(overflowing, required, { safe: false }),
    /Can't cast value 130 to type Int8/,
  )

  // A nullable target takes the value it cannot convert as null under the
  // safe default, and its null rows stay null.
  assert.deepEqual(
    Serie.fromArrowArray(overflowing, fields.int8('quantity')).asJs(),
    [7, null, null],
  )

  // The bits reading shares a same-width buffer instead of converting it.
  const unsigned = arrow.vectorFromArray([2 ** 32 - 1], new arrow.Uint32())
  assert.deepEqual(
    Serie.fromArrowArray(unsigned, fields.int32('digest'), { representation: 'bits' }).asJs(),
    [-1],
  )

  // Each answer is a name, refused by its vocabulary; an unknown key is
  // refused rather than silently doing nothing, and an absent one is skipped.
  assert.throws(
    () => Serie.fromArrowArray(unsigned, fields.int32('digest'), { representation: 'raw' }),
    /expected one of value, bits/,
  )
  assert.throws(
    () => Serie.fromArrowArray(overflowing, required, { nullable: 'strict' }),
    /cast options take safe and representation/,
  )
  assert.throws(
    () =>
      Serie.fromArrowArray(overflowing, required, {
        safe: undefined,
        representation: undefined,
      }),
    /Can't cast value 130 to type Int8/,
  )
})

test('an Arrow JS batch or table lands as the record column of its rows', () => {
  const table = narrow([1, 2], ['AAPL', 'MSFT'])

  const own = Serie.fromArrowBatch(table)
  assert.ok(own instanceof StructSerie)
  assert.equal(own.field.name, 'row')
  assert.deepEqual(own.names, ['id', 'symbol'])

  const root = trades()
  for (const source of [table, table.batches[0]]) {
    const cast = Serie.fromArrowBatch(source, root)
    assert.ok(cast.field.equals(root))
    const batch = cast.intoArrowBatch()
    assert.deepEqual(
      batch.schema.fields.map((field) => `${field.name}: ${field.type}`),
      ['id: Int64', 'symbol: Utf8'],
    )
    assert.deepEqual([...batch.getChild('id')], [1n, 2n])
  }

  // A column the root never declared is dropped; a required one the
  // source cannot fill is refused by path, whatever `safe` says.
  const required = fields.struct(
    'row',
    [Field.from('id: int64'), Field.from('venue: utf8 not null')],
    { nullable: false },
  )
  assert.throws(
    () => Serie.fromArrowBatch(table, required),
    /required Arrow field \$\.venue is missing from the source/,
  )
  assert.throws(() => Serie.fromArrowBatch([], root), TypeError)
})

test('a required field inside a struct is refused by its whole path', () => {
  const target = Field.from(
    'row: struct<account: struct<id: int64, zip: utf8 not null>> not null',
  )
  const identifier = arrow.Field.new('id', new arrow.Int64(), true)
  const postcode = arrow.Field.new('zip', new arrow.Utf8(), true)
  const accounts = (values, children) =>
    new arrow.Table({
      account: arrow.vectorFromArray(values, new arrow.Struct(children)),
    })

  const without = accounts([{ id: 1n }, { id: 2n }], [identifier])
  assert.throws(
    () => Serie.fromArrowBatch(without, target),
    /required Arrow field \$\.account\.zip is missing from the source/,
  )

  const withNull = accounts(
    [{ id: 1n, zip: '75001' }, { id: 2n, zip: null }],
    [identifier, postcode],
  )
  assert.throws(
    () => Serie.fromArrowBatch(withNull, target),
    /required Arrow field \$\.account\.zip holds 1 null values/,
  )
})

test('a native reader drains into one record column, cast by one plan', () => {
  const source = narrow([1, 2], ['AAPL', 'MSFT'])
  const stream = BatchReader.from(source)
  const serie = Serie.fromArrowReader(stream, trades())
  assert.equal(stream.consumed, true)
  assert.ok(serie.field.equals(trades()))
  assert.deepEqual(serie.child('id').asJs(), [1, 2])

  // Its own schema, named `row`, when no root is given.
  const own = Serie.fromArrowReader(BatchReader.from(source))
  assert.equal(own.field.name, 'row')
  assert.equal(own.child('id').field.dtype.toString(), 'int32')

  // A stream is read once, and another Arrow representation is converted by
  // the caller rather than guessed at.
  assert.throws(() => Serie.fromArrowReader(stream, trades()), /already been consumed/)
  assert.throws(() => Serie.fromArrowReader(source), /use BatchReader\.from\(value\)/)
})

test('a cast failure inside a stream reports the failure, not the envelope', () => {
  // A reader can only carry a core failure boxed inside an ArrowError, and
  // that envelope is transport: every door must hand back the failure the
  // cast raised, not `External error: <the real one>`.
  const target = fields.struct('row', [fields.ascii('ccy', { nullable: false })], {
    nullable: false,
  })
  const source = new arrow.Table({
    ccy: arrow.vectorFromArray(['USÉ'], new arrow.Utf8()),
  })
  const strict = { safe: false }

  for (const drain of [
    () => Serie.fromArrowBatch(source, target, strict),
    () => Serie.fromArrowReader(BatchReader.from(source), target, strict),
    () => [...StreamChunkedSerie.fromArrowReader(BatchReader.from(source), target, strict)],
    () =>
      StreamChunkedSerie.fromArrowReader(BatchReader.from(source), target, strict)
        .intoArrowReader()
        .intoIpc(),
  ]) {
    assert.throws(drain, (error) => {
      assert.ok(
        !/External error/.test(error.message),
        `the transport envelope reached the caller: ${error.message}`,
      )
      assert.match(error.message, /expected US-ASCII text, got a non-ASCII byte/)
      return true
    })
  }
  // A required column refuses the cell it cannot convert, by that value,
  // even under the safe default.
  assert.throws(
    () => Serie.fromArrowBatch(source, target),
    /expected US-ASCII text, got a non-ASCII byte/,
  )
})

test('a StreamChunkedSerie yields one record serie per batch under one root', () => {
  const first = narrow([1, 2], ['AAPL', 'MSFT'])
  const second = narrow([3], ['NVDA'])
  const source = new arrow.Table([...first.batches, ...second.batches])
  assert.equal(source.batches.length, 2)

  const stream = BatchReader.from(source)
  const reader = StreamChunkedSerie.fromArrowReader(stream, trades())
  assert.equal(stream.consumed, true)
  assert.ok(reader.field.equals(trades()))

  const series = [...reader]
  assert.equal(series.length, 2)
  assert.ok(series.every((serie) => serie instanceof StructSerie))
  assert.deepEqual(
    series.map((serie) => serie.child('id').asJs()),
    [[1, 2], [3]],
  )
  // Drained here, the reader ends quietly; it still answers its field.
  assert.deepEqual([...reader], [])
  assert.ok(reader.field.equals(trades()))

  // Its own schema, named `row`, when no root is given.
  const own = StreamChunkedSerie.fromArrowReader(BatchReader.from(source))
  assert.equal(own.field.name, 'row')
  assert.equal([...own].length, 2)
})

test('a StreamChunkedSerie hands its stream back as a reader, read once', () => {
  const source = narrow([1, 2], ['AAPL', 'MSFT'])
  const reader = StreamChunkedSerie.fromArrowReader(BatchReader.from(source), trades())
  const batches = reader.intoArrowReader()
  assert.ok(batches instanceof BatchReader)
  assert.ok(batches.field.equals(trades()))
  const table = batches.intoTable()
  assert.deepEqual(
    table.schema.fields.map((field) => `${field.name}: ${field.type}`),
    ['id: Int64', 'symbol: Utf8'],
  )
  assert.throws(() => [...reader], /already been consumed/)
  assert.throws(() => reader.intoArrowReader(), /already been consumed/)

  // A missing required column is a property of the schemas alone, refused
  // where they meet: building the reader, with no batch pulled.
  const required = fields.struct(
    'row',
    [Field.from('id: int64'), Field.from('venue: utf8 not null')],
    { nullable: false },
  )
  assert.throws(
    () => StreamChunkedSerie.fromArrowReader(BatchReader.from(source), required),
    /required Arrow field \$\.venue is missing from the source/,
  )
  // A null is a property of rows, so the reader is built and refuses at the
  // pull that reads it.
  const partial = new arrow.Table({
    id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
    venue: arrow.vectorFromArray(['XNAS', null], new arrow.Utf8()),
  })
  const strict = StreamChunkedSerie.fromArrowReader(BatchReader.from(partial), required)
  assert.throws(() => [...strict], /required Arrow field \$\.venue holds 1 null values/)
  assert.throws(() => new StreamChunkedSerie(), /no `constructor`/)
  assert.throws(
    () => StreamChunkedSerie.fromArrowReader(partial, required),
    /StreamChunkedSerie\.fromArrowReader takes a native BatchReader/,
  )
})

test('a held record column is a stream of the one serie it is', () => {
  const records = Serie.fromArrowBatch(narrow([1, 2], ['AAPL', 'MSFT']), trades())
  const reader = StreamChunkedSerie.fromSerie(records)
  assert.ok(reader.field.equals(trades()))
  const series = [...reader]
  assert.equal(series.length, 1)
  assert.ok(series[0] instanceof StructSerie)
  assert.ok(series[0].equals(records))
  // Drained, it ends quietly, and hands back no second batch.
  assert.deepEqual([...reader], [])

  // Handed back as a reader, the held column is the one batch it is.
  const table = StreamChunkedSerie.fromSerie(records).intoArrowReader().intoTable()
  assert.equal(table.batches.length, 1)
  assert.deepEqual([...table.getChild('symbol')], ['AAPL', 'MSFT'])
})

test('a held column that is not a record comes back under a record root', () => {
  const ids = Serie.fromScalars(fields.int64('id'), [1n, 2n, null])
  const reader = StreamChunkedSerie.fromSerie(ids)
  assert.equal(reader.field.name, 'row')
  assert.equal(reader.field.nullable, false)
  assert.deepEqual(
    Array.from(reader.field.dtype, (child) => child.name),
    ['id'],
  )
  const [records, ...rest] = [...reader]
  assert.deepEqual(rest, [])
  assert.ok(records instanceof StructSerie)
  assert.ok(records.child('id').equals(ids))
})

test('a held record column with an absent row, and a run, are refused', () => {
  const nullable = Field.from('row: struct<id: int64>')
  const absent = Serie.fromScalars(nullable, [{ id: 1n }, null])
  assert.throws(
    () => StreamChunkedSerie.fromSerie(absent),
    /record column "row" holds 1 absent rows, which a table cannot state/,
  )
  assert.throws(() => StreamChunkedSerie.fromSerie(new Serie([1, 2])), /run/)
  assert.throws(
    () => StreamChunkedSerie.fromSerie(narrow([1], ['AAPL'])),
    /StreamChunkedSerie\.fromSerie takes a Serie/,
  )
})

test('a held chunked column is a stream of one record serie per chunk', () => {
  const source = new arrow.Table([
    ...narrow([1, 2], ['AAPL', 'MSFT']).batches,
    ...narrow([3], ['NVDA']).batches,
  ])
  const chunked = ChunkedSerie.fromArrowBatch(source, trades())
  assert.equal(chunked.numChunks, 2)

  const reader = StreamChunkedSerie.fromChunked(chunked)
  assert.ok(reader.field.equals(trades()))
  const series = [...reader]
  assert.equal(series.length, chunked.numChunks)
  assert.ok(series.every((serie) => serie instanceof StructSerie))
  assert.ok(series.every((serie, index) => serie.equals(chunked.chunk(index))))
  assert.deepEqual([...reader], [])

  // Handed back as a reader, each chunk is the one batch it is.
  const table = StreamChunkedSerie.fromChunked(chunked).intoArrowReader().intoTable()
  assert.equal(table.batches.length, chunked.numChunks)
  assert.deepEqual(
    table.batches.map((batch) => batch.numRows),
    [2, 1],
  )
  assert.deepEqual([...table.getChild('symbol')], ['AAPL', 'MSFT', 'NVDA'])

  // A leaf column's chunks are each the one child of a `row` record.
  const leaf = StreamChunkedSerie.fromChunked(chunked.child('id'))
  assert.equal(leaf.field.name, 'row')
  assert.deepEqual(
    [...leaf].map((records) => records.child('id').asJs()),
    [[1, 2], [3]],
  )

  // No chunk is the empty stream of the root.
  const empty = StreamChunkedSerie.fromChunked(ChunkedSerie.empty(trades()))
  assert.ok(empty.field.equals(trades()))
  assert.deepEqual([...empty], [])

  // A record chunk with an absent row is refused, and a chunked serie is
  // the one input this door takes.
  const nullable = Field.from('row: struct<id: int64>')
  const absent = ChunkedSerie.fromSerie(Serie.fromScalars(nullable, [{ id: 1n }, null]))
  assert.throws(
    () => StreamChunkedSerie.fromChunked(absent),
    /record column "row" holds 1 absent rows, which a table cannot state/,
  )
  assert.throws(
    () => StreamChunkedSerie.fromChunked(chunked.intoSerie()),
    /StreamChunkedSerie\.fromChunked takes a ChunkedSerie/,
  )
})

test('a StreamChunkedSerie cast under its own root yields the same records', () => {
  const records = Serie.fromArrowBatch(narrow([1, 2], ['AAPL', 'MSFT']), trades())
  const held = StreamChunkedSerie.fromSerie(records)
  const same = held.cast(trades())
  assert.ok(same instanceof StreamChunkedSerie)
  assert.ok(same.field.equals(trades()))
  const series = [...same]
  assert.equal(series.length, 1)
  assert.ok(series[0] instanceof StructSerie)
  assert.ok(series[0].equals(records))
  // The cast took the reader it was asked of; a stream is read once.
  assert.throws(() => [...held], /already been consumed/)
  assert.throws(() => held.cast(trades()), /already been consumed/)
  assert.throws(() => held.intoArrowReader(), /already been consumed/)

  // A stream under its own root yields every batch as it would have.
  const source = new arrow.Table([
    ...narrow([1, 2], ['AAPL', 'MSFT']).batches,
    ...narrow([3], ['NVDA']).batches,
  ])
  const stream = StreamChunkedSerie.fromArrowReader(BatchReader.from(source), trades()).cast(trades())
  assert.deepEqual(
    [...stream].map((serie) => serie.child('id').asJs()),
    [[1, 2], [3]],
  )

  // An option the cast refuses leaves the reader as it was.
  const kept = StreamChunkedSerie.fromSerie(records)
  assert.throws(() => kept.cast(trades(), { bogus: true }), /got "bogus"/)
  assert.equal([...kept].length, 1)
})

test('a StreamChunkedSerie cast into a wider root casts every record, held or streamed', () => {
  const wide = tradesWith('float64')
  const records = Serie.fromArrowBatch(narrow([1, 2], ['AAPL', 'MSFT']), trades())
  const held = StreamChunkedSerie.fromSerie(records).cast(wide)
  assert.ok(held.field.equals(wide))
  const [cast, ...rest] = [...held]
  assert.deepEqual(rest, [])
  assert.equal(cast.child('id').field.dtype.toString(), 'float64')
  assert.deepEqual(cast.asJs(), [
    { id: 1, symbol: 'AAPL' },
    { id: 2, symbol: 'MSFT' },
  ])

  // A stream opened under its own schema, and one opened under a cast of
  // its own: every batch lands under the wider root as it is pulled.
  const source = new arrow.Table([
    ...narrow([1, 2], ['AAPL', 'MSFT']).batches,
    ...narrow([3], ['NVDA']).batches,
  ])
  for (const reader of [
    StreamChunkedSerie.fromArrowReader(BatchReader.from(source)),
    StreamChunkedSerie.fromArrowReader(BatchReader.from(source), trades()),
  ]) {
    const streamed = reader.cast(wide)
    assert.ok(streamed.field.equals(wide))
    const series = [...streamed]
    assert.deepEqual(
      series.map((serie) => serie.child('id').field.dtype.toString()),
      ['float64', 'float64'],
    )
    assert.deepEqual(
      series.map((serie) => serie.child('id').asJs()),
      [[1, 2], [3]],
    )
  }

  // A DataType is the required `value` field, a record root of that name.
  const typed = StreamChunkedSerie.fromSerie(records).cast(
    DataType.from('struct<id: float64, symbol: utf8>'),
  )
  assert.equal(typed.field.name, 'value')
  assert.deepEqual(
    [...typed].map((serie) => serie.child('id').field.dtype.toString()),
    ['float64'],
  )
})

test('a StreamChunkedSerie cast into a narrower root refuses naming the column', () => {
  const tight = tradesWith('int8')
  const records = Serie.fromArrowBatch(narrow([1, 300], ['AAPL', 'MSFT']), trades())
  // The refusal names the column and the value it could not hold.
  const refusal = /\$\.id\b.*\b300\b/
  // Held records are cast where the cast is asked for.
  assert.throws(() => StreamChunkedSerie.fromSerie(records).cast(tight, { safe: false }), refusal)
  // A stream's batches are cast as they are pulled, landed or handed on.
  const stream = () =>
    StreamChunkedSerie.fromArrowReader(BatchReader.from(narrow([1, 300], ['AAPL', 'MSFT'])), trades())
  const pulled = stream().cast(tight, { safe: false })
  assert.throws(() => [...pulled], refusal)
  const moved = stream().cast(tight, { safe: false }).intoArrowReader()
  assert.throws(() => moved.intoTable(), refusal)
  // Under the safe default the value that does not fit is null.
  assert.deepEqual(
    [...StreamChunkedSerie.fromSerie(records).cast(tight)].map((serie) => serie.child('id').asJs()),
    [[1, null]],
  )
})

test('a cast StreamChunkedSerie hands its stream back under the cast schema', () => {
  const wide = tradesWith('float64')
  const source = new arrow.Table([
    ...narrow([1, 2], ['AAPL', 'MSFT']).batches,
    ...narrow([3], ['NVDA']).batches,
  ])
  for (const reader of [
    StreamChunkedSerie.fromArrowReader(BatchReader.from(source), trades()),
    StreamChunkedSerie.fromSerie(Serie.fromArrowBatch(source, trades())),
  ]) {
    const batches = reader.cast(wide).intoArrowReader()
    assert.ok(batches instanceof BatchReader)
    assert.ok(batches.field.equals(wide))
    const table = batches.intoTable()
    assert.deepEqual(
      table.schema.fields.map((field) => `${field.name}: ${field.type}`),
      ['id: Float64', 'symbol: Utf8'],
    )
    assert.deepEqual([...table.getChild('id')], [1, 2, 3])
  }
})

test('a column casts once under another field, and a run is refused', () => {
  const ids = Serie.fromScalars(fields.int32('id'), [1, 2, null])
  const wide = ids.cast(fields.int64('id'))
  assert.equal(wide.field.dtype.toString(), 'int64')
  assert.deepEqual(wide.asJs(), [1, 2, null])
  assert.ok(ids.cast(ids.field).equals(ids))
  assert.ok(ids.cast('id: int64').field.equals(fields.int64('id')))

  // A DataType target is the required `value` field.
  const present = Serie.fromScalars(fields.int32('id'), [1, 2])
  const typed = present.cast(DataType.from('int64'))
  assert.equal(typed.field.name, 'value')
  assert.equal(typed.field.nullable, false)
  assert.deepEqual(typed.asJs(), [1, 2])

  // A required column refuses a null, by path, whatever `safe` says.
  assert.throws(
    () => ids.cast(DataType.from('int64')),
    /required Arrow field \$\.value holds 1 null values/,
  )
  assert.throws(
    () => ids.cast(fields.int64('id', { nullable: false })),
    /required Arrow field \$\.id holds 1 null values/,
  )
  assert.throws(
    () => ids.cast(fields.int64('id', { nullable: false }), { safe: false }),
    /required Arrow field \$\.id holds 1 null values/,
  )

  const run = new Serie([1, 2])
  assert.equal(run.isColumn, false)
  assert.throws(() => run.cast(fields.int64('id')), /run/)
})

test('a field default is a column of its canonical rows', () => {
  const quantity = fields.int32('quantity', { nullable: false })
  const one = Serie.fromDefault(quantity)
  assert.equal(one.length, 1)
  assert.ok(one.field.equals(quantity))
  assert.equal(one.intoArrowScalar(), 0)

  const three = Serie.fromDefault('note: utf8 not null', 3)
  assert.deepEqual(three.asJs(), ['', '', ''])
  assert.equal(Serie.fromDefault(fields.utf8('note')).intoArrowScalar(), null)
  assert.equal(Serie.fromDefault(quantity, 0).length, 0)

  // A required null field has no default to lay out.
  assert.throws(() => Serie.fromDefault(fields.null('nothing', { nullable: false })))
})

test('an Arrow scalar is exactly one row of a column', () => {
  const answer = Serie.fromScalars(fields.int64('answer'), [42n])
  assert.equal(answer.intoArrowScalar(), 42n)
  assert.throws(
    () => Serie.fromScalars(fields.int64('answer'), [1n, 2n]).intoArrowScalar(),
    /exactly one row, got 2/,
  )
  assert.throws(() => new Serie([1]).intoArrowScalar(), /run/)
})

test('a serie compares against a chunked serie by the rows, on either side', () => {
  const { ChunkedSerie } = binding
  const price = fields.int64('price', { nullable: false })
  const serie = Serie.fromScalars(price, [125n, 126n, 127n])
  const chunked = ChunkedSerie.fromSeries(
    [Serie.fromScalars(price, [125n, 126n]), Serie.fromScalars(price, [127n])],
    price,
  )
  assert.ok(serie.equals(chunked))
  assert.ok(chunked.equals(serie))
  assert.equal(serie.compare(chunked), 0)
  assert.equal(serie.compare(chunked.slice(0, 2)), 1)
  assert.equal(chunked.slice(0, 2).compare(serie), -1)
  // A window compares as the serie of its rows, and is any serie argument.
  const held = Serie.fromScalars(price, [124n, 125n, 126n, 127n])
  assert.ok(serie.equals(held.window(1, 3)))
  assert.equal(serie.compare(held.window(0, 3)), 1)
  assert.deepEqual(held.intoTaken(held.window(0, 0)).asJs(), [])
  assert.deepEqual(
    serie.partitionBy(Serie.fromScalars(Field.from('key: int64 not null'), [0n, 1n, 1n, 0n]).window(1, 3)).length,
    2,
  )
  assert.throws(
    () => serie.equals([125n]),
    /Serie\.equals takes a ChunkedSerie, a Serie or a WindowSerie/,
  )
  assert.equal('_equalsNative' in Serie.prototype, false)
  assert.equal('_compareNative' in Serie.prototype, false)

  // A held record streams under its own name, whichever door hands it out.
  const trades = Field.from('trades: struct<id: int64 not null> not null')
  const records = Serie.fromScalars(trades, [[1n]])
  assert.equal(records.intoArrowReader().field.name, 'trades')
  assert.equal(ChunkedSerie.fromSerie(records).intoArrowReader().field.name, 'trades')
  assert.equal(StreamChunkedSerie.fromSerie(records).field.name, 'trades')
  assert.equal(serie.intoArrowReader().field.name, 'row')
})

// ---------------------------------------------------------------------------
// Ordering, uniqueness and grouping: each case mirrors rust/tests/serie/order.rs
// by name, through the binding's doors - the options an object of descending
// and nullsFirst, indices, masks and keys a Serie or an iterable of values.
// ---------------------------------------------------------------------------

// `values` as an int64 column, `null` an absent row.
const int64Column = (values) =>
  Serie.fromScalars(new Field('price', 'int64', values.includes(null)), values)
// `values` as a utf8 column.
const utf8Column = (values) =>
  Serie.fromScalars(new Field('venue', 'utf8', values.includes(null)), values)
// A record column of `[venue, price]` rows.
const quotes = (rows) =>
  Serie.fromScalars(
    Field.from('quote: struct<venue: utf8 not null, price: int64 not null> not null'),
    rows.map(([venue, price]) => ({ venue, price })),
  )

test('sort indices answer a uint32 index column, stable on both leaves', () => {
  const run = new Serie([3, 1, 2, 1])
  const column = int64Column([3, 1, 2, 1])
  for (const serie of [run, column]) {
    const order = serie.sortIndices()
    assert.equal(order.field.name, 'index')
    assert.equal(order.field.dtype.toString(), 'uint32')
    // The two equal rows keep their order: stable.
    assert.deepEqual(order.asJs(), [1, 3, 2, 0])
    assert.deepEqual(serie.sortIndices({ descending: true }).asJs(), [0, 2, 1, 3])
  }
})

test('absent rows gather to the end the options name', () => {
  const run = new Serie([null, 2, null, 1])
  const column = int64Column([null, 2, null, 1])
  const strings = utf8Column([null, 'b', null, 'a'])
  for (const serie of [run, column, strings]) {
    assert.deepEqual(serie.sortIndices().asJs(), [3, 1, 0, 2])
    assert.deepEqual(serie.sortIndices({ descending: true, nullsFirst: true }).asJs(), [0, 2, 1, 3])
  }
})

test('isSorted reads adjacent rows under the options on both leaves', () => {
  const run = new Serie([1, 1, 2, null])
  const column = int64Column([1, 1, 2, null])
  for (const serie of [run, column]) {
    assert.equal(serie.isSorted(), true)
    assert.equal(serie.isSorted({ descending: true }), false)
    assert.equal(serie.isSorted({ nullsFirst: true }), false)
    assert.equal(serie.intoReversed().isSorted({ descending: true, nullsFirst: true }), true)
  }
  assert.equal(new Serie([]).isSorted(), true)
  assert.equal(int64Column([1]).isSorted({ descending: true, nullsFirst: true }), true)
})

test('uniqueness counts an absent row as one value on both leaves', () => {
  const run = new Serie([1, null, 1, null, 2])
  const column = int64Column([1, null, 1, null, 2])
  for (const serie of [run, column]) {
    assert.equal(serie.isUnique(), false)
    assert.equal(serie.uniqueCount(), 3)
    const unique = serie.intoUnique()
    assert.deepEqual(unique.asJs(), [1, null, 2])
    assert.equal(unique.isUnique(), true)
    assert.equal(String(unique.field), String(serie.field))
    // The serie is as it was.
    assert.equal(serie.length, 5)
  }
  assert.equal(new Serie([1, null]).isUnique(), true)
})

test('intoSorted answers a new serie under the same field and leaves this one', () => {
  const column = int64Column([3, null, 1])
  const sorted = column.intoSorted()
  assert.deepEqual(sorted.asJs(), [1, 3, null])
  assert.ok(sorted.field.equals(column.field))
  assert.equal(sorted.isSorted(), true)
  assert.equal(column.scalar(0).asJs(), 3)

  const run = new Serie(['b', 'a'])
  const runSorted = run.intoSorted()
  assert.deepEqual(runSorted.asJs(), ['a', 'b'])
  assert.equal(runSorted.field, null)
})

test('intoReversed reverses both leaves with their absences', () => {
  assert.deepEqual(int64Column([1, null, 3]).intoReversed().asJs(), [3, null, 1])
  assert.deepEqual(new Serie([1, 2]).intoReversed().asJs(), [2, 1])
  assert.equal(new Serie([]).intoReversed().isEmpty(), true)
})

test('intoTaken reads indices of any integer width and refuses what names no row', () => {
  const column = int64Column([10, 20, 30])
  const run = new Serie([10, 20, 30])
  const byUint32 = Serie.fromScalars(new Field('index', 'uint32', false), [2, 0, 2])
  const byInt64 = new Serie([2, 0, 2])
  const byColumn = int64Column([2, 0, 2])
  for (const serie of [column, run]) {
    // An iterable of integers is the run of them.
    for (const indices of [byUint32, byInt64, byColumn, [2, 0, 2]]) {
      const taken = serie.intoTaken(indices)
      assert.deepEqual(taken.asJs(), [30, 10, 30])
      assert.equal(String(taken.field), String(serie.field))
    }
    assert.equal(serie.intoTaken(new Serie([])).isEmpty(), true)
    for (const [indices, what] of [
      [new Serie([3]), 'past the end'],
      [new Serie([-1]), 'negative'],
      [new Serie([null]), 'absent'],
      [new Serie(['0']), 'text'],
      [int64Column([null]), 'an absent column row'],
    ]) {
      assert.throws(() => serie.intoTaken(indices), /names no row of the 3/, what)
    }
  }
  assert.throws(() => column.intoTaken('0'), /Serie.intoTaken indices must be a Serie or an iterable/)
  assert.throws(() => column.intoTaken(2), /Serie.intoTaken indices must be an iterable/)
})

test('intoFiltered keeps what the mask keeps and an absent mask row keeps nothing', () => {
  const column = int64Column([10, 20, 30])
  const run = new Serie([10, 20, 30])
  const maskRun = new Serie([true, null, true])
  const maskColumn = Serie.fromScalars(new Field('keep', 'boolean', true), [true, null, true])
  for (const serie of [column, run]) {
    for (const mask of [maskRun, maskColumn, [true, null, true]]) {
      const kept = serie.intoFiltered(mask)
      assert.deepEqual(kept.asJs(), [10, 30])
      assert.equal(String(kept.field), String(serie.field))
    }
    assert.throws(() => serie.intoFiltered([true]), /a mask of 1 rows cannot filter the 3 rows/)
    assert.throws(() => serie.intoFiltered([1, 1, 1]), /neither a boolean nor absent/)
  }
})

test('partitionBy retains typed keys and first-occurrence payload order', () => {
const prices = int64Column([1, 2, 3, 4])
  const groups = prices.partitionBy(utf8Column(['a', 'a', 'b', null]))
  assert.ok(groups instanceof binding.KeySeries)
  assert.deepEqual([...groups].map(g => [g.key.asJs(), g.rows.asJs()]), [[['a'], [{price:1}, {price:2}]], [['b'], [{price:3}]], [[null], [{price:4}]]])
  const unsorted = prices.partitionBy(utf8Column(['b', 'a', 'b', 'a']))
  assert.deepEqual([...unsorted].map(g => [g.key.asJs(), g.rows.asJs()]), [[['b'], [{price:1}, {price:3}]], [['a'], [{price:2}, {price:4}]]])
  assert.throws(() => prices.partitionBy(utf8Column(['a'])), /keys|length|rows/)
  assert.equal(int64Column([]).partitionBy(utf8Column([])).length, 0)
})

test('partitionBy accepts selector paths and moves bare key children out', () => {
const { FieldPath } = binding
  const source = quotes([['XNAS', 1], ['XNYS', 2], ['XNAS', 1], ['XNAS', 3]])
  const groups = source.partitionBy([new FieldPath('venue'), new FieldPath('price')])
  assert.equal(groups.length, 3)
  assert.deepEqual(groups.get(0).key.asJs(), ['XNAS', 1])
  assert.equal(groups.get(0).rows.length, 2)
  assert.equal(groups.get(0).serieField.fieldLen, 0)
  assert.ok(groups.field.equals(source.field))
  const venue = source.partitionBy('venue')
  assert.equal(venue.length, 2)
  assert.deepEqual(venue.get(0).rows.child('price').asJs(), [1, 1, 3])
  const external = source.partitionBy(source.child('venue').cast(Field.from('desk: utf8')))
  assert.deepEqual(external.get(0).rows.child('venue').asJs(), ['XNAS', 'XNAS', 'XNAS'])
  assert.throws(() => source.partitionBy([]), /empty|key|column/)
  assert.throws(() => source.partitionBy('tier'), /tier/)
})

test('memorySize counts a column as its slice and a run as its values', () => {
  const column = int64Column(Array.from({ length: 1000 }, (_, index) => index))
  const whole = column.memorySize()
  assert.ok(whole >= 8000, `${whole}`)
  assert.ok(column.slice(0, 10).memorySize() < whole / 10)
  assert.ok(new Serie([1, 2, 3]).memorySize() > 0)
  assert.equal(new Serie([]).memorySize(), 0)
})

test('asSorted rewrites a primitive column in place gathering its absences', () => {
  for (const [options, expected] of [
    [undefined, [1, 2, 3, null, null]],
    [{ descending: true }, [3, 2, 1, null, null]],
    [{ nullsFirst: true }, [null, null, 1, 2, 3]],
    [{ descending: true, nullsFirst: true }, [null, null, 3, 2, 1]],
  ]) {
    const column = int64Column([3, null, 1, null, 2])
    const field = column.field
    assert.strictEqual(column.asSorted(options), column)
    assert.deepEqual(column.asJs(), expected, JSON.stringify(options))
    assert.equal(column.nullCount(), 2)
    assert.ok(column.field.equals(field))
    assert.equal(column.isSorted(options), true)
    assert.equal(column.intoArrowArray().nullCount, 2)
  }
  // No absence: the slice alone; the writes chain.
  const column = int64Column([3, 1, 2])
  column.asSorted().asReversed()
  assert.deepEqual(column.asJs(), [3, 2, 1])
})

test('asSorted on a shared column copies once and leaves the other holder alone', () => {
  const column = int64Column([2, 1])
  const other = column.clone()
  other.asSorted()
  assert.deepEqual(other.asJs(), [1, 2])
  assert.deepEqual(column.asJs(), [2, 1])
})

test('every as write brings a run or a kernel leaf into the state and chains', () => {
  const run = new Serie(['b', null, 'a', 'b'])
  assert.strictEqual(run.asSorted().asUnique().asReversed(), run)
  assert.deepEqual(run.asJs(), [null, 'b', 'a'])
  run.asTaken([2, 1]).asFiltered([false, true])
  assert.deepEqual(run.asJs(), ['b'])

  const strings = utf8Column(['b', null, 'a', 'b'])
  const field = strings.field
  strings.asSorted().asUnique().asReversed()
  assert.deepEqual(strings.asJs(), [null, 'b', 'a'])
  assert.ok(strings.field.equals(field))
  strings.asTaken(new Serie([2, 1])).asFiltered(new Serie([false, true]))
  assert.deepEqual(strings.asJs(), ['b'])
  assert.ok(strings.field.equals(field))
})

test('a refused write leaves the serie as it was', () => {
  const column = int64Column([2, 1])
  assert.throws(() => column.asTaken([5]))
  assert.throws(() => column.asFiltered([]))
  assert.deepEqual(column.asJs(), [2, 1])
})

test('asReversed reverses a primitive column in place with its validity', () => {
  const column = int64Column([1, null, null, 4, 5])
  column.asReversed()
  assert.deepEqual(column.asJs(), [5, 4, null, null, 1])
  assert.equal(column.nullCount(), 2)
})

// Every nested, encoded and viewed layout answers the ladder's lower rungs:
// the row format, and the values' own order.
function nestedColumns() {
  const records = quotes([
    ['XNYS', 2],
    ['XNAS', 2],
    ['XNAS', 1],
    ['XNYS', 2],
  ])
  const lists = Serie.fromScalars(Field.from('item: list<int64>'), [[2, 3], [1], [2, 3], [2]])
  const dictionary = Serie.fromArrowArray(
    arrow.vectorFromArray(
      ['XNYS', 'XNAS', 'XNYS', 'XPAR'],
      new arrow.Dictionary(new arrow.Utf8(), new arrow.Int8()),
    ),
  )
  const views = Serie.fromScalars(new Field('venue', 'utf8_view', false), [
    'XNYS',
    'XNAS',
    'a view longer than twelve bytes',
    'XNYS',
  ])
  const booleans = Serie.fromScalars(new Field('flag', 'boolean', true), [true, false, null, true])
  return [records, lists, dictionary, views, booleans]
}

test('every layout answers every verb through the ladder', () => {
  for (const column of nestedColumns()) {
    const what = String(column.field)
    assert.equal(column.sortIndices().length, 4, what)
    const sorted = column.intoSorted()
    assert.equal(sorted.isSorted(), true, what)
    // A whole-row sort of a record declares every column on the root
    // (`SORT:by`); the datatype is the column's.
    assert.ok(sorted.field.dtype.equals(column.field.dtype), what)
    assert.equal(sorted.declaredOrder() !== null, column instanceof StructSerie, what)
    assert.equal(sorted.constructor, column.constructor, what)
    // The sort agrees with the values' own order, absences last.
    const byValue = column.rows().sort((left, right) => {
      const [leftNull, rightNull] = [left.asJs() === null, right.asJs() === null]
      if (leftNull || rightNull) return Number(leftNull) - Number(rightNull)
      return left.compare(right)
    })
    assert.ok(sorted.equals(new Serie(byValue)), what)
    assert.equal(column.intoSorted({ descending: true }).isSorted({ descending: true }), true, what)
    assert.equal(column.isUnique(), false, what)
    assert.equal(column.uniqueCount(), 3, what)
    const unique = column.intoUnique()
    assert.equal(unique.length, 3, what)
    assert.equal(unique.isUnique(), true, what)
    assert.ok(unique.scalar(0).equals(column.scalar(0)), what)
    assert.ok(column.intoReversed().scalar(0).equals(column.scalar(3)), what)
    const groups = column.partitionBy(Serie.fromScalars(Field.from('key: int64 not null'), [0n, 1n, 2n, 0n]))
    assert.equal(groups.length, 3, what)
    assert.equal(groups.get(0).rows.length, 2, what)
    const written = column.clone()
    written.asSorted().asUnique().asReversed()
    assert.equal(written.length, 3, what)
    assert.ok(written.field.dtype.equals(column.field.dtype), what)
    // Ascending with absences last, reversed, is descending with absences
    // first.
    assert.equal(written.isSorted({ descending: true, nullsFirst: true }), true, what)
    assert.ok(column.memorySize() > 0, what)
  }
})

test('the ordering options are validated, skipped when absent and cleared by null', () => {
  const column = int64Column([2, null, 1])
  // `null` and an absent option both take the default.
  assert.deepEqual(column.intoSorted({ descending: null, nullsFirst: null }).asJs(), [1, 2, null])
  assert.deepEqual(column.intoSorted(null).asJs(), [1, 2, null])
  assert.deepEqual(column.intoSorted({}).asJs(), [1, 2, null])
  assert.throws(
    () => column.isSorted({ descending: true, nullsLast: true }),
    /Serie.isSorted options take descending and nullsFirst, got "nullsLast"/,
  )
  assert.throws(() => column.asSorted({ descending: 'yes' }), /option descending must be a boolean/)
  assert.throws(() => column.sortIndices([true]), /options must be an object/)
  // A refused option leaves the serie as it was.
  assert.deepEqual(column.asJs(), [2, null, 1])
})

test('the ordering natives stay outside the public surface', () => {
  for (const name of [
    '_sortIndicesNative',
    '_isSortedNative',
    '_intoSortedNative',
    '_intoUniqueNative',
    '_intoReversedNative',
    '_intoTakenNative',
    '_intoFilteredNative',
    '_partitionByNative',
    '_partitionByPathsNative',
    '_asSortedNative',
    '_asUniqueNative',
    '_asReversedNative',
    '_asTakenNative',
    '_asFilteredNative',
    '_sortIndicesByNative',
    '_intoSortByNative',
    '_asSortByNative',
    '_joinWithNative',
    '_windowNative',
    '_windowByNative',
  ]) {
    assert.equal(name in Serie.prototype, false, name)
  }
  for (const name of ['_intoSortedNative', '_intoSortByNative', '_joinWithNative']) {
    assert.equal(name in StreamChunkedSerie.prototype, false, name)
  }
  // A nested answer is handed out as its leaf's class.
  const records = quotes([['XNAS', 1]])
  for (const answer of [
    records.intoSorted(),
    records.intoUnique(),
    records.intoReversed(),
    records.intoTaken([0]),
    records.intoFiltered([true]),
    records.partitionBy(Serie.fromScalars(Field.from('key: utf8 not null'), ['a'])).get(0).rows,
  ]) {
    assert.ok(answer instanceof StructSerie)
  }
})

// ---------------------------------------------------------------------------
// windowBy: the rows cut where the key changes - mirrors the window_by cases
// of rust/tests/serie/order.rs and the reader's of rust/tests/serie/arrow.rs
// ---------------------------------------------------------------------------

const MINUTE_NS = 60_000_000_000n

// The record `quote{venue, count, ts}`, its root nullable so a row may be
// absent.
const quoteField = () =>
  new Field('quote', 'struct<venue: utf8, count: int64 not null, ts: timestamp(ns, UTC)>', true)

// Quotes, each its venue, its count and its instant in minutes; `null` an
// absent row.
const quoteColumn = (rows) =>
  Serie.fromScalars(
    quoteField(),
    rows.map((row) =>
      row === null ? null : { venue: row[0], count: row[1], ts: BigInt(row[2]) * MINUTE_NS },
    ),
  )

// The venues XNAS, XNAS, XNYS, XNAS at minutes 0, 14, 15 and 31.
const venueRuns = () =>
  quoteColumn([
    ['XNAS', 1, 0],
    ['XNAS', 2, 14],
    ['XNYS', 3, 15],
    ['XNAS', 4, 31],
  ])

const windowCuts = (windows) =>
  windows.map(([key, window]) => [key.asJs(), window.offset, window.length])

// Every window's record as its natural JavaScript value.
const staticRecords = (windows) => windows.map((window) => window.staticValues.asJs())

const childNames = (field) =>
  Array.from({ length: field.fieldLen }, (_, index) => field.getFieldAt(index).name)

const readerRows = (window) => [...window].flatMap((piece) => piece.asJs())

test('windowBy validates selectors and permits ordinary rownum aliases', () => {
for (const sorted of [false, true]) {
    for (const source of [venueRuns(), quoteColumn([])]) {
      assert.throws(() => source.windowBy('venue,', sorted), /expected/)
      assert.throws(() => source.windowBy([], sorted), /empty|column|key/)
      assert.throws(() => source.windowBy('tier', sorted), /tier/)
      assert.throws(() => source.windowBy('unnest(items)', sorted), /key/)
      assert.ok(source.windowBy('count as rownum', sorted) instanceof binding.KeySeries)
      assert.ok(source.windowBy('count as windownum', sorted) instanceof binding.KeySeries)
    }
  }
  assert.throws(() => venueRuns().windowBy('venue', 1), /boolean/)
  assert.throws(() => venueRuns().windowBy(3), /selector|Selector|typed/)
})

test('windowBy cuts adjacent keys and states their absolute rownum', () => {
const source = venueRuns()
  const expected = [[['XNAS'], 0, 2], [['XNYS'], 2, 1], [['XNAS'], 3, 1]]
  for (const by of ['venue', ['venue'], new Selector('venue'), Term.column('venue'), 'VENUE']) {
    for (const sorted of [false, undefined, null]) {
      const groups = source.windowBy(by, sorted)
      assert.deepEqual([...groups].map(g => [g.key.asJs(), g.rownum, g.rows.length]), expected)
      for (const g of groups) {
        assert.ok(g instanceof binding.KeySerie)
        assert.equal(Boolean(g.serieField.getField('venue')), false)
        assert.deepEqual(childNames(g.field).map(name => name.toLowerCase()), childNames(source.field))
      }
    }
  }
  assert.equal(quoteColumn([]).windowBy('venue').length, 0)
  assert.deepEqual([...source.windowBy('venue, minutes(ts, 15) as bucket')].map(g => g.key.asJs()), [['XNAS', 0], ['XNYS', 1], ['XNAS', 2]])
})

test('windowBy distinguishes absent parent rows and absent key cells', () => {
const source = quoteColumn([['XNAS', 1, 0], null, null, [null, 4, 0], [null, 5, 0]])
  assert.deepEqual([...source.windowBy('venue')].map(g => [g.key.asJs(), g.rownum, g.rows.length]), [[['XNAS'], 0, 1], [[null], 1, 2], [[null], 3, 2]])
})

test('sorted windows regroup keys and clear rownum after a gather', () => {
const source = quoteColumn([['XNYS', 1, 0], ['XNAS', 2, 0], ['XNYS', 3, 0], null, ['XNAS', 5, 0]])
  const groups = source.windowBy('venue', true)
  assert.deepEqual([...groups].map(g => [g.key.asJs(), g.rownum, g.rows.length]), [[['XNAS'], null, 2], [['XNYS'], null, 2], [[null], null, 1]])
  assert.deepEqual(groups.get(0).rows.child('count').asJs(), [2, 5])
  const ordered = venueRuns().windowBy('minutes(ts, 15)', true)
  assert.deepEqual([...ordered].map(g => g.rownum), [0, 2, 3])
})

test('a primitive column keeps its typed key and empty payload rows', () => {
const source = Serie.fromScalars(new Field('venue', 'utf8', false), ['XNAS', 'XNAS', 'XNYS'])
  const groups = source.windowBy('venue')
  assert.deepEqual([...groups].map(g => [g.key.asJs(), g.rownum, g.rows.length]), [[['XNAS'], 0, 2], [['XNYS'], 2, 1]])
  assert.deepEqual([...groups.intoStream()].map(row => row.asJs()), [['XNAS'], ['XNAS'], ['XNYS']])
})

test('stream window preflight validates before pulling rows', () => {
const reader = StreamChunkedSerie.fromSerie(venueRuns())
  assert.throws(() => reader.windowBy('venue,'), /expected/)
  assert.throws(() => reader.windowBy('venue', 1), /boolean/)
  assert.equal([...reader].length, 1)
  for (const key of ['tier', 'unnest(items)', '*']) {
    const source = StreamChunkedSerie.fromSerie(venueRuns())
    assert.throws(() => source.windowBy(key), /tier|key|column/)
  }
})

test('stream windows borrow native payload pieces across batch edges', () => {
const source = venueRuns()
  const walk = StreamChunkedSerie.fromChunked(ChunkedSerie.fromSeries([source.slice(0, 1), source.slice(1, 3)], source.field)).windowBy('venue')
  assert.ok(walk instanceof StreamKeySerie)
  assert.strictEqual(walk[Symbol.iterator](), walk)
  const first = walk.next().value
  assert.ok(first instanceof binding.KeySerie)
  assert.deepEqual(first.key.asJs(), ['XNAS'])
  assert.equal(first.rownum, 0)
  assert.deepEqual([...first.rows.intoChunkedStream(1, null)].map(piece => piece.length), [1, 1])
  const second = walk.next().value
  assert.deepEqual(second.rows.child('count').asJs(), [3])
  const tail = walk.next().value
  assert.equal(tail.rownum, 3)
  assert.equal([...tail.rows.intoStream()].length, 1)
  assert.equal(walk.next().done, true)
})

test('stream windows refuse payloads after their cursor advances', () => {
const walk = StreamChunkedSerie.fromSerie(venueRuns()).windowBy('venue')
  const first = walk.next().value
  walk.next()
  assert.throws(() => [...first.rows.intoStream()], /window|passed|advance/)
})

test('sorted stream windows report descending keys and fuse the failure', () => {
const source = quoteColumn([['XLON', 1, 0], ['XNYS', 2, 0], ['XNAS', 3, 0]])
  const walk = StreamChunkedSerie.fromSerie(source).windowBy('venue', true)
  assert.equal([...walk.next().value.rows.intoStream()].length, 1)
  assert.equal([...walk.next().value.rows.intoStream()].length, 1)
  assert.throws(() => walk.next(), /order|batch.*row/)
  assert.equal(walk.next().done, true)
  assert.deepEqual([...StreamChunkedSerie.fromSerie(source).windowBy('venue')].map(g => g.key.asJs()), [['XLON'], ['XNYS'], ['XNAS']])
})

test('held and streamed windows share key fields and absolute rownums', () => {
const source = venueRuns()
  const held = [...source.windowBy('venue')].map(g => [g.key.asJs(), g.rownum])
  const streamed = [...StreamChunkedSerie.fromSerie(source).windowBy('venue')].map(g => [g.key.asJs(), g.rownum])
  assert.deepEqual(streamed, held)
  const groups = source.windowBy('minutes(ts, 30) as half')
  const inner = groups.get(0).windowBy('venue')
  assert.deepEqual([...inner].map(g => [g.key.asJs(), g.rownum]), [[[0, 'XNAS'], 0], [[0, 'XNYS'], 2]])
})

test('key context and payload fields compose without static metadata', () => {
const source = venueRuns()
  const group = StreamChunkedSerie.fromSerie(source).windowBy('venue').next().value
  assert.equal(group.keyField.fieldLen, 1)
  assert.deepEqual(group.key.asJs(), ['XNAS'])
  assert.equal(Boolean(group.serieField.getField('venue')), false)
  assert.equal(group.field.fieldLen, group.keyField.fieldLen + group.serieField.fieldLen)
  assert.equal('staticValues' in group, false)
  const table = group.intoChunkedStream().intoArrowReader().intoTable()
  assert.deepEqual(table.schema.fields.map(f => f.name), ['venue', 'count', 'ts'])
  assert.equal(table.numRows, 2)
})

// ---------------------------------------------------------------------------
// partitionBy: a stream's rows cut by key, each partition yielded as it
// closes - mirrors rust/tests/serie/partition.rs
// ---------------------------------------------------------------------------

// The record `row{venue, qty}`, every row present.
const venueQtyField = () =>
  new Field('row', 'struct<venue: utf8 not null, qty: int64 not null>', false)

// One batch of `[venue, qty]` rows.
const venueBatch = (rows, field = venueQtyField()) =>
  Serie.fromScalars(
    field,
    rows.map(([venue, qty]) => ({ venue, qty })),
  )

// One stream of `batches` under `field`, a chunk per batch.
const venueStream = (batches, field = venueQtyField()) =>
  StreamChunkedSerie.fromChunked(
    ChunkedSerie.fromSeries(
      batches.map((rows) => venueBatch(rows, field)),
      field,
    ),
  )

// Every partition in the order it closes: its key, and the quantities of
// its rows in the order they are held.
const closedPartitions = (partitions) =>
  Array.from(partitions, group => [group.key.asJs(), group.rows.asJs().map(row => row.qty)])

test('StreamChunkedSerie.partitionBy reads its key and options before the reader is taken', () => {
  // A key that does not parse, and options no partitioning reads, leave the
  // reader usable.
  const reader = venueStream([[['XNAS', 1]]])
  assert.throws(() => reader.partitionBy('venue,'), /expected a value or a name/)
  assert.throws(() => reader.partitionBy(3), {
    name: 'TypeError',
    message: /selector text|Selector|typed external keys/,
  })
  assert.throws(() => reader.partitionBy('venue', 2), {
    name: 'TypeError',
    message: /StreamChunkedSerie(?:\.prototype)?\.partitionBy options must be an object of maxOpen, threads and clustered/,
  })
  assert.throws(() => reader.partitionBy('venue', { maxopen: 2 }), {
    name: 'TypeError',
    message: /StreamChunkedSerie(?:\.prototype)?\.partitionBy options take maxOpen, threads and clustered, got "maxopen"/,
  })
  for (const maxOpen of [-1, 1.5, Number.NaN]) {
    assert.throws(
      () => reader.partitionBy('venue', { maxOpen }),
      /maxOpen must be a non-negative whole number/,
    )
  }
  assert.throws(
    () => reader.partitionBy('venue', { threads: -1 }),
    /threads must be a non-negative whole number/,
  )
  assert.throws(() => reader.partitionBy('venue', { clustered: 'yes' }))
  assert.deepEqual(readerRows(reader), [{ venue: 'XNAS', qty: 1 }])
  // A key the root refuses consumes it, as a refused windowBy does, before
  // any batch is pulled.
  for (const [refused, reason] of [
    ['missing', /missing/],
    ['*', /empty match key/],
  ]) {
    const spent = venueStream([[['XNAS', 1]]])
    assert.throws(() => spent.partitionBy(refused), reason)
    assert.throws(() => [...spent], /already been consumed/)
    assert.throws(() => spent.partitionBy('venue'), /already been consumed/)
  }
  // The walk has no public constructor, and its bridges are hidden.
  assert.throws(() => new StreamKeySerie(), /constructor|construct|handed|returned/)
  assert.equal('_nextNative' in StreamKeySerie.prototype, false)
  assert.equal('_partitionByNative' in StreamChunkedSerie.prototype, false)
})

test('an unbounded stream holds every partition and closes them in key order', () => {
  const stream = venueStream([
    [
      ['XPAR', 1],
      ['XNAS', 2],
    ],
    [
      ['XLON', 3],
      ['XNAS', 4],
    ],
  ])
  const root = stream.field
  const walk = stream.partitionBy('venue')
  assert.ok(walk instanceof StreamKeySerie)
  assert.strictEqual(walk[Symbol.iterator](), walk)
  // The root every partition is held under is the reader's own, known
  // before a batch is pulled.
  assert.ok(walk.field.equals(root))
  assert.throws(() => [...stream], /already been consumed/)
  const first = walk.next()
  assert.equal(first.done, false)
  const {key, rows} = first.value
  assert.deepEqual(key.asJs(), ['XLON'])
  assert.ok(rows instanceof Serie)
  assert.equal(Boolean(rows.field.getField('venue')), false)
  assert.deepEqual(rows.asJs(), [{ qty: 3 }])
  // A partition's rows are the chunks the batches gave it, in arrival order.
  const xnas = walk.next().value.rows
  assert.equal([...xnas.intoChunkedStream(1, null)].length, 2)
  assert.deepEqual(
    xnas.asJs().map((row) => row.qty),
    [2, 4],
  )
  assert.deepEqual(walk.next().value.key.asJs(), ['XPAR'])
  assert.deepEqual(walk.next(), { done: true, value: undefined })
  assert.deepEqual([...walk], [])
  // Options absent, `undefined` or `null` are the default.
  const batches = [
    [
      ['XPAR', 1],
      ['XNAS', 2],
    ],
    [['XLON', 3]],
  ]
  const expected = [
    [['XLON'], [3]],
    [['XNAS'], [2]],
    [['XPAR'], [1]],
  ]
  for (const options of [
    undefined,
    null,
    {},
    { maxOpen: undefined, threads: undefined, clustered: undefined },
    { maxOpen: null, threads: null, clustered: null },
  ]) {
    assert.deepEqual(
      closedPartitions(venueStream(batches).partitionBy('venue', options)),
      expected,
    )
  }
  // Any key the windows take: a Selector, a Term, an array of them.
  assert.deepEqual(
    closedPartitions(venueStream(batches).partitionBy(new Selector('venue'))),
    expected,
  )
  assert.deepEqual(
    closedPartitions(venueStream(batches).partitionBy([Term.column('venue')])),
    expected,
  )
})

test('past maxOpen the lowest keys close and a returning key opens a new piece', () => {
  const batches = [
    [
      ['XNAS', 1],
      ['XLON', 2],
    ],
    [['XPAR', 3]],
    [['XLON', 4]],
  ]
  // The third venue closes the lowest open, XLON; XLON returning is the
  // lowest open again and closes at once; the rest close at the end, in key
  // order.
  assert.deepEqual(closedPartitions(venueStream(batches).partitionBy('venue', { maxOpen: 2 })), [
    [['XLON'], [2]],
    [['XLON'], [4]],
    [['XNAS'], [1]],
    [['XPAR'], [3]],
  ])
  // A bound of none is a bound of one.
  assert.deepEqual(
    closedPartitions(venueStream(batches).partitionBy('venue', { maxOpen: 0 })),
    closedPartitions(venueStream(batches).partitionBy('venue', { maxOpen: 1 })),
  )
})

test('a clustered stream closes each partition once another key arrives', () => {
  const batches = [
    [
      ['XNAS', 1],
      ['XNAS', 2],
      ['XLON', 3],
    ],
    [
      ['XLON', 4],
      ['XPAR', 5],
    ],
    [['XNAS', 6]],
  ]
  // In arrival order, a run across a batch edge one partition, and a key the
  // stream returns to a second piece of it: pieces, never rows.
  const walk = venueStream(batches).partitionBy('venue', { clustered: true })
  const pieces = Array.from(walk, group => [group.key.asJs(), [...group.rows.intoChunkedStream(1, null)].length, group.rows.length])
  assert.deepEqual(pieces, [
    [['XNAS'], 1, 2],
    [['XLON'], 2, 2],
    [['XPAR'], 1, 1],
    [['XNAS'], 1, 1],
  ])
  // Unclustered, every partition is held to the end and closed in key order.
  for (const clustered of [false, null, undefined]) {
    assert.deepEqual(closedPartitions(venueStream(batches).partitionBy('venue', { clustered })), [
      [['XLON'], [3, 4]],
      [['XNAS'], [1, 2, 6]],
      [['XPAR'], [5]],
    ])
  }
})

test('a root declaring an order that leads with the key is clustered untold', () => {
  // Venue descending, then quantity: the declaration the sort writes, kept
  // by every slice and proven at every chunk edge where the chunks land.
  const sorted = venueBatch([
    ['XLON', 4],
    ['XNAS', 3],
    ['XPAR', 1],
    ['XNAS', 2],
  ]).intoSortBy('venue desc, qty')
  const declaring = sorted.field
  assert.notEqual(sorted.declaredOrder(), null)
  const stream = StreamChunkedSerie.fromChunked(
    ChunkedSerie.fromSeries([sorted.slice(0, 2), sorted.slice(2, 2)], declaring),
  )
  assert.ok(stream.field.equals(declaring))
  // The partitions close in the order they arrive, venue descending, where an
  // undeclared stream of the same rows closes in key order.
  assert.deepEqual(closedPartitions(stream.partitionBy('venue')), [
    [['XPAR'], [1]],
    [['XNAS'], [2, 3]],
    [['XLON'], [4]],
  ])
  const undeclared = venueStream([
    [
      ['XPAR', 1],
      ['XNAS', 2],
    ],
    [
      ['XNAS', 3],
      ['XLON', 4],
    ],
  ])
  assert.deepEqual(closedPartitions(undeclared.partitionBy('venue')), [
    [['XLON'], [4]],
    [['XNAS'], [2, 3]],
    [['XPAR'], [1]],
  ])
})

test('partitions cut on many threads are the ones cut on one', () => {
  const venues = ['XNAS', 'XLON', 'XPAR', 'XAMS', 'XETR']
  const batches = Array.from({ length: 40 }, (_, index) =>
    Array.from({ length: 25 }, (_, row) => {
      const position = index * 25 + row
      return [venues[position % 5], position]
    }),
  )
  for (const options of [{}, { maxOpen: 2 }, { clustered: true }]) {
    const one = closedPartitions(
      venueStream(batches).partitionBy('venue', { ...options, threads: 1 }),
    )
    const many = closedPartitions(
      venueStream(batches).partitionBy('venue', { ...options, threads: 4 }),
    )
    assert.deepEqual(one, many, JSON.stringify(options))
    assert.equal(
      one.reduce((rows, [, quantities]) => rows + quantities.length, 0),
      1_000,
      JSON.stringify(options),
    )
  }
  // Zero threads is one.
  assert.deepEqual(
    closedPartitions(venueStream(batches).partitionBy('venue', { threads: 0 })),
    closedPartitions(venueStream(batches).partitionBy('venue', { threads: 1 })),
  )
})

// ---------------------------------------------------------------------------
// Spill: where the rows live, and the one verb that moves them to disk.
// ---------------------------------------------------------------------------

test('spill moves a column to disk and every read reaches the mapping', () => {
  const prices = Serie.fromScalars(
    new Field('price', 'int64', false),
    Array.from({ length: 1_024 }, (_, index) => index),
  )
  assert.equal(prices.residentSize(), prices.memorySize())
  assert.equal(prices.isSpilled(), false)
  const before = prices.clone()
  assert.equal(prices.spill(new SpillOptions({ byteSize: 0 })), undefined)
  assert.equal(prices.isSpilled(), true)
  assert.equal(prices.residentSize(), 0)
  assert.ok(prices.memorySize() > 0)
  assert.ok(prices.equals(before))
  assert.equal(prices.scalar(7).asJs(), 7)
  // The clone taken before keeps its heap bytes.
  assert.equal(before.isSpilled(), false)
  // A write brings the rows it touches back to the heap, once.
  prices.push(1_024)
  assert.equal(prices.isSpilled(), false)
  assert.equal(prices.length, 1_025)
})

test('spill leaves a run, a never bound and a column under the default alone', () => {
  const run = new Serie([1, 2, 3])
  run.spill(new SpillOptions({ byteSize: 0 }))
  assert.equal(run.isSpilled(), false)
  assert.equal(run.residentSize(), run.memorySize())
  const prices = int64Column([3, 1, 2])
  prices.spill(new SpillOptions({ byteSize: SpillOptions.NEVER }))
  assert.equal(prices.isSpilled(), false)
  // Absent and null are the process default, 64 MiB unless the environment
  // says otherwise: three rows stay resident.
  prices.spill()
  prices.spill(null)
  assert.equal(prices.isSpilled(), false)
  assert.equal(prices.residentSize(), prices.memorySize())
})

test('a record spills child by child, heaviest first, and reads back whole', () => {
  const root = Field.from('quote: struct<venue: utf8 not null, price: int64 not null> not null')
  const rows = Array.from({ length: 512 }, (_, index) => ({
    venue: ['XNAS', 'XNYS'][index % 2],
    price: index,
  }))
  const quotes = Serie.fromScalars(root, rows)
  const before = quotes.clone()
  quotes.spill(new SpillOptions({ byteSize: 0 }))
  assert.equal(quotes.residentSize(), 0)
  assert.ok(quotes.equals(before))
  assert.deepEqual(quotes.asJs()[3], { venue: 'XNYS', price: 3 })
})

test('asSpilled spills in place and answers the serie, intoSpilled spills a copy', () => {
  const zero = new SpillOptions({ byteSize: 0 })
  const prices = Serie.fromScalars(
    new Field('price', 'int64', false),
    Array.from({ length: 1_024 }, (_, index) => index),
  )
  const copy = prices.intoSpilled(zero)
  assert.ok(copy instanceof Serie)
  assert.equal(copy.isSpilled(), true)
  assert.equal(copy.residentSize(), 0)
  assert.equal(prices.isSpilled(), false)
  assert.ok(copy.equals(prices))

  // In place, so calls chain.
  assert.equal(prices.asSpilled(zero), prices)
  assert.equal(prices.isSpilled(), true)
  assert.equal(prices.asSpilled(zero).asReversed().scalar(0).asJs(), 1_023)

  // Absent and null are the process default, which leaves three rows resident.
  const small = int64Column([3, 1, 2])
  assert.equal(small.asSpilled(), small)
  assert.equal(small.asSpilled(null).isSpilled(), false)
  assert.equal(small.intoSpilled().isSpilled(), false)
  // A record keeps its leaf class across the copy.
  const records = quotes([['XNAS', 1], ['XNYS', 2]])
  const spilledRecords = records.intoSpilled(zero)
  assert.ok(spilledRecords instanceof StructSerie)
  assert.deepEqual(spilledRecords.names, ['venue', 'price'])
  assert.equal(spilledRecords.residentSize(), 0)
  assert.throws(() => small.asSpilled({ byteSize: 0 }), /SpillOptions/)
})

test('lit holds one value for every row, laid out only when exported', () => {
  const field = Field.from('venue: utf8 not null')
  const venue = Serie.lit(field, 'XNAS', 1_000_000)
  assert.ok(venue instanceof Serie)
  assert.equal(venue.isLit, true)
  assert.equal(venue.length, 1_000_000)
  assert.ok(venue.field.equals(field))
  assert.equal(venue.scalar(999_999).asJs(), 'XNAS')
  const resident = venue.residentSize()
  assert.ok(resident < 1_024, 'one row, not a million')
  // The laid-out estimate, answered without laying the column out.
  assert.ok(venue.memorySize() > resident)
  assert.equal(venue.residentSize(), resident, 'memorySize builds no array')
  // A spill forgets the built array and writes nothing: a lit never spills.
  assert.equal(venue.asSpilled(new SpillOptions({ byteSize: 0 })).isSpilled(), false)

  // A slice moves the count and stays a constant.
  const three = venue.slice(10, 3)
  assert.equal(three.isLit, true)
  assert.deepEqual(three.asJs(), ['XNAS', 'XNAS', 'XNAS'])
  // A constant is the rows it is: equal to the laid-out column, and exported as it.
  const laid = Serie.fromScalars(field, ['XNAS', 'XNAS', 'XNAS'])
  assert.equal(laid.isLit, false)
  assert.ok(three.equals(laid))
  assert.deepEqual(three.intoArrowArray().toArray(), ['XNAS', 'XNAS', 'XNAS'])

  // A write of the same value keeps it a constant; another value lays it out.
  three.push('XNAS')
  assert.equal(three.isLit, true)
  assert.equal(three.length, 4)
  three.set(0, 'XNYS')
  assert.equal(three.isLit, false)
  assert.deepEqual(three.asJs(), ['XNYS', 'XNAS', 'XNAS', 'XNAS'])

  // The canonical default is a constant too, and a field expression is a field.
  const zeros = Serie.fromDefault(Field.from('price: int64 not null'), 3)
  assert.equal(zeros.isLit, true)
  assert.deepEqual(zeros.asJs(), [0, 0, 0])
  assert.deepEqual(Serie.lit('price: int64', 7, 2).asJs(), [7, 7])
  // A text field reads an integer as the text it spells.
  assert.deepEqual(Serie.lit('venue: utf8', 42, 2).asJs(), ['42', '42'])
  assert.equal(Serie.lit('price: int64', null, 2).nullCount(), 2)
  assert.equal(Serie.lit(field, 'XNAS', 0).length, 0)
})

test('lit refuses what the field refuses', () => {
  assert.throws(
    () => Serie.lit(Field.from('venue: utf8 not null'), null, 3),
    /\$\.venue: non-nullable field received null/,
  )
  assert.throws(
    () => Serie.lit(Field.from('price: int64 not null'), 'XNAS', 3),
    /\$\.price: expected int64, got string/,
  )
  assert.throws(() => Serie.lit(Field.from('venue: utf8'), [1, 2], 2), /expected utf8, got serie/)
  for (const length of [-1, 1.5, Number.NaN]) {
    assert.throws(() => Serie.lit(Field.from('venue: utf8'), 'XNAS', length), /length must be/)
  }
})

// ---------------------------------------------------------------------------
// Orderings by key: `by` read once by the core's own `order by` key rule.
// ---------------------------------------------------------------------------

const keyedQuotes = () =>
  Serie.fromScalars(
    Field.from('quote: struct<venue: utf8 not null, price: int64> not null'),
    [
      { venue: 'XNYS', price: 1 },
      { venue: 'XNAS', price: 2 },
      { venue: 'XNAS', price: null },
      { venue: 'XNYS', price: 3 },
    ],
  )

test('sortIndicesBy reads every spelling of the keys as one', () => {
  const quotes = keyedQuotes()
  for (const by of [
    'venue, price desc nulls first',
    ['venue', 'price desc nulls first'],
    [{ term: 'venue' }, { term: 'price', descending: true, nulls_first: true }],
  ]) {
    const order = quotes.sortIndicesBy(by)
    assert.equal(order.field.name, 'index')
    assert.equal(order.field.dtype.toString(), 'uint32')
    assert.deepEqual(order.asJs(), [2, 1, 3, 0], JSON.stringify(by))
  }
  // One record is one key; a Selector is every projection ascending.
  assert.deepEqual(quotes.sortIndicesBy({ term: 'price', descending: true }).asJs(), [3, 1, 0, 2])
  assert.deepEqual(quotes.sortIndicesBy(new Selector('venue')).asJs(), [1, 2, 0, 3])
  // A plain column keys as itself, under its own name.
  assert.deepEqual(int64Column([3, 1, 2]).sortIndicesBy('price desc').asJs(), [0, 2, 1])
})

test('sortIndicesBy refuses what names no key, before any row is read', () => {
  const quotes = keyedQuotes()
  // The record's flag is read under either spelling, a boolean or the text
  // the crate's one boolean table reads; anything else is refused by key.
  assert.deepEqual(
    quotes.sortIndicesBy([{ term: 'price', nullsFirst: true }]).asJs(),
    quotes.sortIndicesBy([{ term: 'price', nulls_first: true }]).asJs(),
  )
  assert.deepEqual(
    quotes.sortIndicesBy([{ term: 'price', nullsFirst: 'yes' }]).asJs(),
    quotes.sortIndicesBy([{ term: 'price', nullsFirst: true }]).asJs(),
  )
  assert.throws(() => quotes.sortIndicesBy([{ term: 'price', nullsFirst: 'maybe' }]), /\$\.nullsFirst/)
  assert.throws(() => quotes.sortIndicesBy([{ descending: true }]), /names the term it orders by/)
  assert.throws(() => quotes.sortIndicesBy('tier'))
  assert.throws(() => quotes.sortIndicesBy([]), /at least one `order by` key/)
  assert.throws(() => new Serie([1, 2]).sortIndicesBy('price'), /sorts by no term/)
})

test('intoSortBy declares the keys it sorted by and leaves the serie alone', () => {
  const quotes = keyedQuotes()
  assert.equal(quotes.declaredOrder(), null)
  const sorted = quotes.intoSortBy('venue desc, price desc')
  assert.ok(sorted instanceof StructSerie)
  assert.deepEqual(sorted.asJs(), [
    { venue: 'XNYS', price: 3 },
    { venue: 'XNYS', price: 1 },
    { venue: 'XNAS', price: 2 },
    { venue: 'XNAS', price: null },
  ])
  assert.deepEqual(sorted.declaredOrder(), ['venue desc', 'price desc'])
  assert.equal(sorted.field.get('SORT:by'), '["venue desc","price desc"]')
  assert.deepEqual(quotes.asJs()[0], { venue: 'XNYS', price: 1 })
  // What the declaration states is answered without a pass.
  assert.deepEqual(sorted.sortIndicesBy('venue desc').asJs(), [0, 1, 2, 3])
  // A whole-row sort declares every column; a column that is no record
  // declares nothing.
  assert.deepEqual(quotes.intoSorted().declaredOrder(), ['venue', 'price'])
  assert.equal(int64Column([2, 1]).intoSortBy('price').declaredOrder(), null)
})

test('asSortBy sorts in place, chains, and a refusal leaves the serie', () => {
  const prices = int64Column([2, 3, 1])
  assert.strictEqual(prices.asSortBy('price desc').asReversed(), prices)
  assert.deepEqual(prices.asJs(), [1, 2, 3])
  assert.throws(() => prices.asSortBy('tier'))
  assert.deepEqual(prices.asJs(), [1, 2, 3])
})

// ---------------------------------------------------------------------------
// Joins: two held columns matched on key terms.
// ---------------------------------------------------------------------------

const tradeRows = () =>
  Serie.fromScalars(
    Field.from('trade: struct<id: int64 not null, venue: utf8 not null> not null'),
    [
      { id: 1, venue: 'XNAS' },
      { id: 2, venue: 'XNYS' },
    ],
  )
const venueRows = () =>
  Serie.fromScalars(
    Field.from('venue: struct<venue: utf8 not null, city: utf8 not null> not null'),
    [{ venue: 'XNAS', city: 'New York' }],
  )

test('joinWith answers the left columns then the right, a shared key once', () => {
  const joined = tradeRows().joinWith(venueRows(), 'venue', 'left')
  assert.ok(joined instanceof StructSerie)
  assert.equal(joined.field.name, 'trade')
  assert.deepEqual(joined.names, ['id', 'venue', 'city'])
  assert.deepEqual(joined.asJs(), [
    { id: 1, venue: 'XNAS', city: 'New York' },
    { id: 2, venue: 'XNYS', city: null },
  ])
  // The kind is `inner` when absent or null.
  for (const how of [undefined, null, 'inner', 'INNER JOIN']) {
    assert.deepEqual(tradeRows().joinWith(venueRows(), 'venue', how).asJs(), [
      { id: 1, venue: 'XNAS', city: 'New York' },
    ])
  }
  // The filtering kinds answer the left columns alone.
  assert.deepEqual(tradeRows().joinWith(venueRows(), 'venue', 'semi').asJs(), [
    { id: 1, venue: 'XNAS' },
  ])
  assert.deepEqual(tradeRows().joinWith(venueRows(), 'venue', 'anti').asJs(), [
    { id: 2, venue: 'XNYS' },
  ])
  // A window joins as the serie of its rows.
  const trades = tradeRows()
  assert.equal(trades.window(1, 1).joinWith, undefined)
  assert.deepEqual(venueRows().joinWith(trades.window(0, 1), 'venue').asJs(), [
    { venue: 'XNAS', city: 'New York', id: 1 },
  ])
})

test('joinWith refuses a run, a key naming no column and anything but a serie', () => {
  assert.throws(() => tradeRows().joinWith(new Serie([1]), 'venue'))
  assert.throws(() => tradeRows().joinWith(venueRows(), 'tier'))
  assert.throws(() => tradeRows().joinWith(venueRows(), 'venue', 'cross'), /one of `inner`/)
  assert.throws(
    () => tradeRows().joinWith([{ venue: 'XNAS' }], 'venue'),
    /Serie\.joinWith takes a Serie or a WindowSerie/,
  )
  assert.throws(() => tradeRows().joinWith(venueRows(), 'venue', 7), /how must be a join kind/)
})

// ---------------------------------------------------------------------------
// The stream: held records spill, a stream sorts by draining and merging,
// and joins one probe batch at a time.
// ---------------------------------------------------------------------------

test('a reader spills the records it holds and a stream holds none', () => {
  const prices = Serie.fromScalars(
    new Field('price', 'int64', false),
    Array.from({ length: 256 }, (_, index) => index),
  )
  const held = StreamChunkedSerie.fromSerie(prices)
  assert.ok(held.residentSize() > 0)
  assert.equal(held.isSpilled(), false)
  held.spill(new SpillOptions({ byteSize: 0 }))
  assert.equal(held.isSpilled(), true)
  assert.equal(held.residentSize(), 0)
  const [record] = [...held]
  assert.deepEqual(record.child('price').asJs().slice(0, 3), [0, 1, 2])
  // A stream holds no landed batch between pulls.
  const stream = StreamChunkedSerie.fromArrowReader(BatchReader.from(narrow([1], ['AAPL'])), trades())
  assert.equal(stream.residentSize(), 0)
  assert.equal(stream.isSpilled(), false)
  stream.spill(new SpillOptions({ byteSize: 0 }))
  assert.equal([...stream].length, 1)
  const taken = StreamChunkedSerie.fromSerie(prices)
  taken.intoArrowReader()
  assert.throws(() => taken.spill(), /already been consumed/)
})

const readRows = (reader) => [...reader].flatMap((serie) => serie.asJs())

test('a reader spills in place, or hands its records over spilled and is consumed', () => {
  const zero = new SpillOptions({ byteSize: 0 })
  const prices = Serie.fromScalars(
    new Field('price', 'int64', false),
    Array.from({ length: 256 }, (_, index) => index),
  )
  const held = StreamChunkedSerie.fromSerie(prices)
  assert.equal(held.asSpilled(zero), held)
  assert.equal(held.isSpilled(), true)
  assert.equal([...held][0].child('price').length, 256)

  const source = StreamChunkedSerie.fromSerie(prices)
  const moved = source.intoSpilled(zero)
  assert.ok(moved instanceof StreamChunkedSerie)
  assert.notEqual(moved, source)
  assert.equal(moved.isSpilled(), true)
  assert.ok(moved.field.equals(source.field))
  assert.throws(() => [...source], /already been consumed/)
  assert.throws(() => source.intoSpilled(zero), /already been consumed/)
  assert.throws(() => source.asSpilled(zero), /already been consumed/)
  assert.equal([...moved][0].child('price').scalar(255).asJs(), 255)

  // A stream holds no landed batch: spilling it moves it untouched.
  const stream = StreamChunkedSerie.fromArrowReader(BatchReader.from(narrow([1], ['AAPL'])), trades())
  const streamed = stream.intoSpilled()
  assert.equal([...streamed].length, 1)
})


test('a reader sorts by draining and merging, its root kept and the reader consumed', () => {
  const held = () => quotes([['XNYS', 2], ['XNAS', 1], ['XNYS', 1]])
  const reader = StreamChunkedSerie.fromSerie(held())
  const sorted = reader.intoSorted({ descending: true })
  assert.ok(sorted instanceof StreamChunkedSerie)
  assert.equal(sorted.field.name, 'quote')
  assert.deepEqual(readRows(sorted), [
    { venue: 'XNYS', price: 2 },
    { venue: 'XNYS', price: 1 },
    { venue: 'XNAS', price: 1 },
  ])
  assert.throws(() => [...reader], /already been consumed/)
  assert.deepEqual(readRows(StreamChunkedSerie.fromSerie(held()).intoSortBy('venue, price desc')), [
    { venue: 'XNAS', price: 1 },
    { venue: 'XNYS', price: 2 },
    { venue: 'XNYS', price: 1 },
  ])
  // A key no column answers is refused with no batch pulled, the reader
  // consumed; options are refused before anything is.
  const refused = StreamChunkedSerie.fromSerie(held())
  assert.throws(() => refused.intoSortBy('tier'))
  assert.throws(() => refused.intoSortBy('venue'), /already been consumed/)
  const untouched = StreamChunkedSerie.fromSerie(held())
  assert.throws(() => untouched.intoSorted({ order: 'desc' }), /descending and nullsFirst/)
  assert.equal(readRows(untouched).length, 3)
})

test('a reader joins a held column, a chunked one or another stream, consuming both', () => {
  const expected = [
    { id: 1, venue: 'XNAS', city: 'New York' },
    { id: 2, venue: 'XNYS', city: null },
  ]
  for (const other of [
    venueRows(),
    ChunkedSerie.fromSerie(venueRows()),
    StreamChunkedSerie.fromSerie(venueRows()),
  ]) {
    const stream = StreamChunkedSerie.fromSerie(tradeRows())
    const joined = stream.joinWith(other, 'venue', 'left')
    assert.ok(joined instanceof StreamChunkedSerie)
    assert.equal(joined.field.name, 'trade')
    assert.deepEqual(readRows(joined), expected)
    assert.throws(() => [...stream], /already been consumed/)
    if (other instanceof StreamChunkedSerie) {
      assert.throws(() => [...other], /already been consumed/)
    }
  }
  // The kind and the options are read before anything is consumed.
  const kept = StreamChunkedSerie.fromSerie(tradeRows())
  assert.throws(() => kept.joinWith(venueRows(), 'venue', 'cross'), /one of `inner`/)
  assert.throws(() => kept.joinWith(venueRows(), 'venue', 'left', { how: 'left' }), /got "how"/)
  assert.throws(() => kept.joinWith(kept, 'venue'), /cannot join a stream with itself/)
  assert.throws(
    () => kept.joinWith([{ venue: 'XNAS' }], 'venue'),
    /takes a Serie, a ChunkedSerie or a StreamChunkedSerie/,
  )
  assert.equal(readRows(kept).length, 2)
})
