'use strict'

// Pins node/src/serie.rs: the Serie Arrow doors, the one cast a column takes,
// and the SerieReader stream, each redirected into the core with the three
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
  Serie,
  SerieReader,
  StructSerie,
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
  assert.equal(Object.hasOwn(SerieReader, '_fromArrowReaderNative'), false)
  assert.equal(Object.hasOwn(SerieReader, '_fromSerieNative'), false)
  assert.equal(Object.hasOwn(SerieReader, '_fromChunkedNative'), false)
  assert.equal('_nextNative' in SerieReader.prototype, false)
  assert.equal('_castNative' in SerieReader.prototype, false)
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

test('the three cast answers reach the core', () => {
  const overflowing = arrow.vectorFromArray([7, 130, null], new arrow.Int32())
  const required = fields.int8('quantity', { nullable: false })

  // Safe by default: the value the target cannot hold is null, and the
  // required column repairs every absent row with its default.
  assert.deepEqual(Serie.fromArrowArray(overflowing, required).asJs(), [7, 0, 0])

  // Strict nullability refuses what a required column cannot hold: a value
  // it cannot convert by that value, before the absent row by its path.
  assert.throws(
    () => Serie.fromArrowArray(overflowing, required, { nullability: 'strict' }),
    /Can't cast value 130 to type Int8/,
  )

  // `safe: false` refuses the conversion itself, whatever the policy.
  for (const nullability of ['default', 'strict']) {
    assert.throws(
      () => Serie.fromArrowArray(overflowing, required, { safe: false, nullability }),
      /Can't cast value 130 to type Int8/,
    )
  }

  // The bits reading shares a same-width buffer instead of converting it.
  const unsigned = arrow.vectorFromArray([2 ** 32 - 1], new arrow.Uint32())
  assert.deepEqual(
    Serie.fromArrowArray(unsigned, fields.int32('digest'), { representation: 'bits' }).asJs(),
    [-1],
  )

  // Each answer is a name, refused by its vocabulary; an unknown key is
  // refused rather than silently doing nothing, and an absent one is skipped.
  assert.throws(
    () => Serie.fromArrowArray(overflowing, required, { nullability: 'lenient' }),
    /expected one of default, strict/,
  )
  assert.throws(
    () => Serie.fromArrowArray(unsigned, fields.int32('digest'), { representation: 'raw' }),
    /expected one of value, bits/,
  )
  assert.throws(
    () => Serie.fromArrowArray(overflowing, required, { nullable: 'strict' }),
    /cast options take safe, nullability and representation/,
  )
  assert.deepEqual(
    Serie.fromArrowArray(overflowing, required, {
      safe: undefined,
      nullability: undefined,
      representation: undefined,
    }).asJs(),
    [7, 0, 0],
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

  // A column the root never declared is dropped; a required one the source
  // cannot fill is repaired by default and refused by path when strict.
  const required = fields.struct(
    'row',
    [Field.from('id: int64'), Field.from('venue: utf8 not null')],
    { nullable: false },
  )
  assert.deepEqual(Serie.fromArrowBatch(table, required).child('venue').asJs(), ['', ''])
  assert.throws(
    () => Serie.fromArrowBatch(table, required, { nullability: 'strict' }),
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
  assert.deepEqual(
    Serie.fromArrowBatch(without, target)
      .child('account')
      .child('zip')
      .asJs(),
    ['', ''],
  )
  assert.throws(
    () => Serie.fromArrowBatch(without, target, { nullability: 'strict' }),
    /required Arrow field \$\.account\.zip is missing from the source/,
  )

  const withNull = accounts(
    [{ id: 1n, zip: '75001' }, { id: 2n, zip: null }],
    [identifier, postcode],
  )
  assert.throws(
    () => Serie.fromArrowBatch(withNull, target, { nullability: 'strict' }),
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
    () => [...SerieReader.fromArrowReader(BatchReader.from(source), target, strict)],
    () =>
      SerieReader.fromArrowReader(BatchReader.from(source), target, strict)
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
  // Under the safe default the failing cell is null, which the non-null
  // column fills with its default rather than raising.
  assert.deepEqual(Serie.fromArrowBatch(source, target).child('ccy').asJs(), [''])
})

test('a SerieReader yields one record serie per batch under one root', () => {
  const first = narrow([1, 2], ['AAPL', 'MSFT'])
  const second = narrow([3], ['NVDA'])
  const source = new arrow.Table([...first.batches, ...second.batches])
  assert.equal(source.batches.length, 2)

  const stream = BatchReader.from(source)
  const reader = SerieReader.fromArrowReader(stream, trades())
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
  const own = SerieReader.fromArrowReader(BatchReader.from(source))
  assert.equal(own.field.name, 'row')
  assert.equal([...own].length, 2)
})

test('a SerieReader hands its stream back as a reader, read once', () => {
  const source = narrow([1, 2], ['AAPL', 'MSFT'])
  const reader = SerieReader.fromArrowReader(BatchReader.from(source), trades())
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
    () => SerieReader.fromArrowReader(BatchReader.from(source), required, { nullability: 'strict' }),
    /required Arrow field \$\.venue is missing from the source/,
  )
  // A null is a property of rows, so the reader is built and refuses at the
  // pull that reads it.
  const partial = new arrow.Table({
    id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
    venue: arrow.vectorFromArray(['XNAS', null], new arrow.Utf8()),
  })
  const strict = SerieReader.fromArrowReader(BatchReader.from(partial), required, {
    nullability: 'strict',
  })
  assert.throws(() => [...strict], /required Arrow field \$\.venue holds 1 null values/)
  assert.throws(() => new SerieReader(), /no `constructor`/)
  assert.throws(
    () => SerieReader.fromArrowReader(partial, required),
    /SerieReader\.fromArrowReader takes a native BatchReader/,
  )
})

test('a held record column is a stream of the one serie it is', () => {
  const records = Serie.fromArrowBatch(narrow([1, 2], ['AAPL', 'MSFT']), trades())
  const reader = SerieReader.fromSerie(records)
  assert.ok(reader.field.equals(trades()))
  const series = [...reader]
  assert.equal(series.length, 1)
  assert.ok(series[0] instanceof StructSerie)
  assert.ok(series[0].equals(records))
  // Drained, it ends quietly, and hands back no second batch.
  assert.deepEqual([...reader], [])

  // Handed back as a reader, the held column is the one batch it is.
  const table = SerieReader.fromSerie(records).intoArrowReader().intoTable()
  assert.equal(table.batches.length, 1)
  assert.deepEqual([...table.getChild('symbol')], ['AAPL', 'MSFT'])
})

test('a held column that is not a record comes back under a record root', () => {
  const ids = Serie.fromScalars(fields.int64('id'), [1n, 2n, null])
  const reader = SerieReader.fromSerie(ids)
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
    () => SerieReader.fromSerie(absent),
    /record column "row" holds 1 absent rows, which a table cannot state/,
  )
  assert.throws(() => SerieReader.fromSerie(new Serie([1, 2])), /run/)
  assert.throws(
    () => SerieReader.fromSerie(narrow([1], ['AAPL'])),
    /SerieReader\.fromSerie takes a Serie/,
  )
})

test('a held chunked column is a stream of one record serie per chunk', () => {
  const source = new arrow.Table([
    ...narrow([1, 2], ['AAPL', 'MSFT']).batches,
    ...narrow([3], ['NVDA']).batches,
  ])
  const chunked = ChunkedSerie.fromArrowBatch(source, trades())
  assert.equal(chunked.numChunks, 2)

  const reader = SerieReader.fromChunked(chunked)
  assert.ok(reader.field.equals(trades()))
  const series = [...reader]
  assert.equal(series.length, chunked.numChunks)
  assert.ok(series.every((serie) => serie instanceof StructSerie))
  assert.ok(series.every((serie, index) => serie.equals(chunked.chunk(index))))
  assert.deepEqual([...reader], [])

  // Handed back as a reader, each chunk is the one batch it is.
  const table = SerieReader.fromChunked(chunked).intoArrowReader().intoTable()
  assert.equal(table.batches.length, chunked.numChunks)
  assert.deepEqual(
    table.batches.map((batch) => batch.numRows),
    [2, 1],
  )
  assert.deepEqual([...table.getChild('symbol')], ['AAPL', 'MSFT', 'NVDA'])

  // A leaf column's chunks are each the one child of a `row` record.
  const leaf = SerieReader.fromChunked(chunked.child('id'))
  assert.equal(leaf.field.name, 'row')
  assert.deepEqual(
    [...leaf].map((records) => records.child('id').asJs()),
    [[1, 2], [3]],
  )

  // No chunk is the empty stream of the root.
  const empty = SerieReader.fromChunked(ChunkedSerie.empty(trades()))
  assert.ok(empty.field.equals(trades()))
  assert.deepEqual([...empty], [])

  // A record chunk with an absent row is refused, and a chunked serie is
  // the one input this door takes.
  const nullable = Field.from('row: struct<id: int64>')
  const absent = ChunkedSerie.fromSerie(Serie.fromScalars(nullable, [{ id: 1n }, null]))
  assert.throws(
    () => SerieReader.fromChunked(absent),
    /record column "row" holds 1 absent rows, which a table cannot state/,
  )
  assert.throws(
    () => SerieReader.fromChunked(chunked.intoSerie()),
    /SerieReader\.fromChunked takes a ChunkedSerie/,
  )
})

test('a SerieReader cast under its own root yields the same records', () => {
  const records = Serie.fromArrowBatch(narrow([1, 2], ['AAPL', 'MSFT']), trades())
  const held = SerieReader.fromSerie(records)
  const same = held.cast(trades())
  assert.ok(same instanceof SerieReader)
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
  const stream = SerieReader.fromArrowReader(BatchReader.from(source), trades()).cast(trades())
  assert.deepEqual(
    [...stream].map((serie) => serie.child('id').asJs()),
    [[1, 2], [3]],
  )

  // An option the cast refuses leaves the reader as it was.
  const kept = SerieReader.fromSerie(records)
  assert.throws(() => kept.cast(trades(), { bogus: true }), /got "bogus"/)
  assert.throws(() => kept.cast(trades(), { nullability: 'lenient' }), /nullability/)
  assert.equal([...kept].length, 1)
})

test('a SerieReader cast into a wider root casts every record, held or streamed', () => {
  const wide = tradesWith('float64')
  const records = Serie.fromArrowBatch(narrow([1, 2], ['AAPL', 'MSFT']), trades())
  const held = SerieReader.fromSerie(records).cast(wide)
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
    SerieReader.fromArrowReader(BatchReader.from(source)),
    SerieReader.fromArrowReader(BatchReader.from(source), trades()),
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
  const typed = SerieReader.fromSerie(records).cast(
    DataType.from('struct<id: float64, symbol: utf8>'),
  )
  assert.equal(typed.field.name, 'value')
  assert.deepEqual(
    [...typed].map((serie) => serie.child('id').field.dtype.toString()),
    ['float64'],
  )
})

test('a SerieReader cast into a narrower root refuses naming the column', () => {
  const tight = tradesWith('int8')
  const records = Serie.fromArrowBatch(narrow([1, 300], ['AAPL', 'MSFT']), trades())
  // The refusal names the column and the value it could not hold.
  const refusal = /\$\.id\b.*\b300\b/
  // Held records are cast where the cast is asked for.
  assert.throws(() => SerieReader.fromSerie(records).cast(tight, { safe: false }), refusal)
  // A stream's batches are cast as they are pulled, landed or handed on.
  const stream = () =>
    SerieReader.fromArrowReader(BatchReader.from(narrow([1, 300], ['AAPL', 'MSFT'])), trades())
  const pulled = stream().cast(tight, { safe: false })
  assert.throws(() => [...pulled], refusal)
  const moved = stream().cast(tight, { safe: false }).intoArrowReader()
  assert.throws(() => moved.intoTable(), refusal)
  // Under the safe default the value that does not fit is null.
  assert.deepEqual(
    [...SerieReader.fromSerie(records).cast(tight)].map((serie) => serie.child('id').asJs()),
    [[1, null]],
  )
})

test('a cast SerieReader hands its stream back under the cast schema', () => {
  const wide = tradesWith('float64')
  const source = new arrow.Table([
    ...narrow([1, 2], ['AAPL', 'MSFT']).batches,
    ...narrow([3], ['NVDA']).batches,
  ])
  for (const reader of [
    SerieReader.fromArrowReader(BatchReader.from(source), trades()),
    SerieReader.fromSerie(Serie.fromArrowBatch(source, trades())),
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
  const typed = ids.cast(DataType.from('int64'))
  assert.equal(typed.field.name, 'value')
  assert.equal(typed.field.nullable, false)
  assert.deepEqual(typed.asJs(), [1, 2, 0])

  assert.deepEqual(ids.cast(fields.int64('id', { nullable: false })).asJs(), [1, 2, 0])
  assert.throws(
    () => ids.cast(fields.int64('id', { nullable: false }), { nullability: 'strict' }),
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
  assert.throws(() => serie.equals([125n]), /Serie.equals takes a ChunkedSerie or a Serie/)
  assert.equal('_equalsNative' in Serie.prototype, false)
  assert.equal('_compareNative' in Serie.prototype, false)

  // A held record streams under its own name, whichever door hands it out.
  const trades = Field.from('trades: struct<id: int64 not null> not null')
  const records = Serie.fromScalars(trades, [[1n]])
  assert.equal(records.intoArrowReader().field.name, 'trades')
  assert.equal(ChunkedSerie.fromSerie(records).intoArrowReader().field.name, 'trades')
  assert.equal(SerieReader.fromSerie(records).field.name, 'trades')
  assert.equal(serie.intoArrowReader().field.name, 'row')
})
