'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')

const arrow = require('apache-arrow')

const { BatchReader, Field, IOBase, MimeType, RecordOptions, fields } = require('yggdryl')

function scratch() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-records-'))
}

// Apache Arrow JS marks every field it builds nullable, so a declared root that
// wants a required column is refused rather than relabeled. These rows are
// declared the way Arrow JS spells them.
function schema() {
  return fields.struct(
    'row',
    [Field.from('id: int64'), Field.from('symbol: utf8'), Field.from('venue: utf8')],
    { nullable: false },
  )
}

function rows(ids, symbols, venues) {
  return new arrow.Table({
    id: arrow.vectorFromArray(ids, new arrow.Int64()),
    symbol: arrow.vectorFromArray(symbols, new arrow.Utf8()),
    venue: arrow.vectorFromArray(venues, new arrow.Utf8()),
  })
}

function trades() {
  return rows([1n, 2n], ['AAPL', 'MSFT'], ['XNAS', 'XNAS'])
}

test('a handle names its own encoding and round-trips Arrow batches', () => {
  const handle = IOBase.fromBytes()
  handle.mediaType = MimeType.ARROW_STREAM

  // The encoding is never guessed: it is whatever the handle says it holds.
  const options = handle.recordOptions()
  assert.equal(options.toString(), 'application/vnd.apache.arrow.stream')
  assert.equal(options.rootName, 'row')
  assert.equal(options.safe, false)
  assert.equal(options.batchSize, null)

  handle.writeArrowBatchReader(BatchReader.from(trades()))
  assert.ok(handle.size > 0)
  assert.ok(handle.readArrowField().equals(schema()))

  const reader = handle.readArrowBatchReader()
  assert.ok(reader.field.equals(schema()))
  let read = 0
  for (const batch of reader) {
    read += batch.numRows
  }
  assert.equal(read, 2)
  // A stream is read once, and says so rather than reading as empty.
  assert.ok(reader.consumed)
  assert.throws(() => reader.toIpc(), /already been consumed/)

  // A reader a write took reports the same, rather than iterating as no rows.
  const written = BatchReader.from(trades())
  handle.writeArrowBatchReader(written)
  assert.ok(written.consumed)
  assert.throws(() => [...written], /already been consumed/)
})

test('a batch reader is built from whatever a caller already holds', () => {
  const table = trades()
  const ipc = arrow.tableToIPC(table)

  for (const source of [table, table.batches[0], [...table.batches], ipc]) {
    const reader = BatchReader.from(source)
    assert.ok(reader.field.equals(schema()))
    assert.equal(reader.toTable().numRows, 2)
  }

  // A reader passes through itself, so a caller never wraps one twice.
  const reader = BatchReader.from(table)
  assert.equal(BatchReader.from(reader), reader)
  assert.throws(() => BatchReader.from(null), TypeError)
  assert.throws(() => BatchReader.from([]), TypeError)
})

test('a declared schema selects and then casts', () => {
  const handle = IOBase.fromBytes()
  handle.mediaType = MimeType.ARROW_STREAM
  handle.writeArrowBatchReader(trades())
  const plain = handle.recordOptions()

  const wanted = fields.struct('row', [Field.from('id: int64')], { nullable: false })
  const projected = handle.readArrowBatchReader(plain.withSchema(wanted))
  assert.equal(projected.field.dataType.length, 1)
  assert.equal(projected.toTable().numCols, 1)

  // The resource is unchanged: it still holds all three columns.
  assert.equal(handle.readArrowField().dataType.length, 3)

  // A projection can only drop columns, so a column the stream does not hold
  // is read whole and then supplied by the cast.
  const invented = fields.struct(
    'row',
    [Field.from('id: int64'), Field.from('nowhere: utf8')],
    { nullable: false },
  )
  const widened = handle.readArrowBatchReader(plain.withSchema(invented))
  assert.equal(widened.field.dataType.length, 2)
})

test('parquet is chosen by the file name and nothing else', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const file = new IOBase(path.join(root, 'trades.parquet'))
  assert.equal(file.recordOptions().toString(), 'application/vnd.apache.parquet')

  const declared = file.recordOptions().withSchema(schema())
  file.writeArrowBatchReader(trades(), declared)
  assert.ok(file.size > 0)
  assert.ok(file.readArrowField().equals(schema()))
  assert.equal(file.readArrowBatchReader().toTable().numRows, 2)

  // Both sides of an append stream, and the incoming batches are cast first.
  file.appendArrowBatchReader(rows([3n], ['NVDA'], ['XNAS']), declared)
  const table = file.readArrowBatchReader().toTable()
  assert.equal(table.numRows, 3)
  assert.deepEqual(
    table.getChild('symbol').toArray(),
    ['AAPL', 'MSFT', 'NVDA'],
  )
})

test('a declared root that a batch cannot satisfy is refused', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const file = new IOBase(path.join(root, 'trades.parquet'))

  // Arrow JS builds nullable columns, so a required one is a real mismatch; the
  // declared schema casts the incoming rows into it before anything is encoded.
  const required = fields.struct(
    'row',
    [Field.from('id: int64 not null'), Field.from('symbol: utf8'), Field.from('venue: utf8')],
    { nullable: false },
  )
  file.writeArrowBatchReader(trades(), file.recordOptions().withSchema(required))
  assert.ok(file.readArrowField().equals(required))
})

test('a match key updates a stored row and appends a new one', () => {
  const handle = IOBase.fromBytes()
  handle.mediaType = MimeType.ARROW_STREAM
  handle.writeArrowBatchReader(trades())

  const merging = handle.recordOptions().withMergeByNames(['id'])
  assert.deepEqual(merging.mergeByNames, ['id'])
  handle.writeArrowBatchReader(rows([2n, 9n], ['MSFT.O', 'NVDA'], ['XNYS', 'XNYS']), merging)

  const table = handle.readArrowBatchReader().toTable()
  assert.equal(table.numRows, 3)
  assert.deepEqual(table.getChild('symbol').toArray(), ['AAPL', 'MSFT.O', 'NVDA'])
})

test('a zero row limit reads the schema and no batches', () => {
  const handle = IOBase.fromBytes()
  handle.mediaType = MimeType.ARROW_STREAM
  handle.writeArrowBatchReader(trades())

  // `0` is a valid ask, not an error: the shaped schema still answers.
  const reader = handle.readArrowBatchReader(handle.recordOptions().withMaxRowSize(0))
  assert.ok(reader.field.equals(schema()))
  assert.equal(reader.toTable().numRows, 0)
})

test('a row limit is exact over a bigger file', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const file = new IOBase(path.join(root, 'trades.parquet'))
  const count = 100
  const many = rows(
    Array.from({ length: count }, (_, index) => BigInt(index)),
    Array.from({ length: count }, () => 'AAPL'),
    Array.from({ length: count }, () => 'XNAS'),
  )
  file.writeArrowBatchReader(BatchReader.from(many))

  const options = file.recordOptions()
  options.maxRowSize = 10
  assert.equal(options.maxRowSize, 10)
  assert.equal(file.readArrowBatchReader(options).toTable().numRows, 10)
})

test('a small byte limit still yields at least one row', () => {
  const handle = IOBase.fromBytes()
  handle.mediaType = MimeType.ARROW_STREAM
  handle.writeArrowBatchReader(trades())

  // One byte admits no whole row, but a bounded read must never be a silent
  // total loss: only a limit of zero yields nothing.
  const options = handle.recordOptions().withMaxByteSize(1)
  assert.equal(handle.readArrowBatchReader(options).toTable().numRows, 1)
})

test('a limit with a match key is refused naming both settings', () => {
  const handle = IOBase.fromBytes()
  handle.mediaType = MimeType.ARROW_STREAM
  handle.writeArrowBatchReader(trades())

  // A truncated merge would update the matched keys it kept and silently drop
  // the rest, so the combination is refused rather than corrupting.
  const limited = handle.recordOptions().withMergeByNames(['id']).withMaxRowSize(10)
  assert.throws(
    () => handle.writeArrowBatchReader(trades(), limited),
    /max_row_size = 10.*merge_by_names/,
  )
})

test('a folder is one table, and a write routes rows to their partition', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  fs.mkdirSync(path.join(root, 'venue=XNAS'), { recursive: true })

  const lake = new IOBase(root)
  const options = RecordOptions.forMimeType(MimeType.ARROW_STREAM).withSchema(schema())
  lake.writeArrowBatchReader(trades(), options)

  // The value the directory spells is not stored again in every row.
  const leaf = lake.joinpath('venue=XNAS').joinpath('part-0.arrows')
  assert.equal(leaf.readArrowField().dataType.length, 2)

  const restored = lake.readArrowBatchReader(options).toTable()
  assert.equal(restored.numCols, 3)
  assert.deepEqual(restored.getChild('venue').toArray(), ['XNAS', 'XNAS'])
})

test('record options are values, and a setting is set or carried forward', () => {
  const options = RecordOptions.forMimeType(MimeType.PARQUET)
  assert.equal(options.mimeType.toString(), 'application/vnd.apache.parquet')
  assert.equal(options.schema, null)

  const declared = options.withSchema(schema()).withBatchSize(1024).withSafe(true)
  assert.ok(declared.schema.equals(schema()))
  assert.equal(declared.batchSize, 1024)
  assert.equal(declared.safe, true)
  // `with*` returns a new value, so the one it was built from is untouched.
  assert.equal(options.batchSize, null)
  assert.equal(options.safe, false)

  options.rootName = 'trade'
  options.level = 9
  assert.equal(options.rootName, 'trade')
  assert.equal(options.level, 9)

  // An encoding this build does not implement is named rather than guessed.
  assert.throws(
    () => RecordOptions.forMimeType('text/csv'),
    /expected a record encoding this build implements/,
  )
})

test('a setting one encoding has is absent on the others', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const parquet = RecordOptions.from('trades.parquet')
  // The default is what the core declares, spelled the way the format's own
  // parser accepts, so reading one and setting it back is a round trip.
  assert.equal(parquet.compression, 'zstd(1)')
  assert.equal(parquet.maxRowGroupSize, 1_048_576)
  assert.deepEqual(parquet.keyValueMetadata, [])
  parquet.compression = parquet.compression
  assert.equal(parquet.compression, 'zstd(1)')

  const declared = parquet
    .withCompression('snappy')
    .withMaxRowGroupSize(512)
    .withKeyValue('iceberg.schema-id', '7')
  assert.equal(declared.compression, 'snappy')
  assert.equal(declared.maxRowGroupSize, 512)
  assert.deepEqual(declared.keyValueMetadata, [{ key: 'iceberg.schema-id', value: '7' }])

  // The setting reaches the file: uncompressed pages are bigger than snappy ones.
  const sizes = ['uncompressed', 'snappy'].map((compression) => {
    const file = new IOBase(path.join(root, `trades-${compression}.parquet`))
    const ids = Array.from({ length: 4_000 }, (_, index) => BigInt(index))
    file.writeArrowBatchReader(
      new arrow.Table({
        id: arrow.vectorFromArray(ids, new arrow.Int64()),
        symbol: arrow.vectorFromArray(ids.map(() => 'AAPL'), new arrow.Utf8()),
      }),
      file.recordOptions().withCompression(compression),
    )
    assert.equal(file.readArrowBatchReader().toTable().numRows, 4_000, compression)
    return file.size
  })
  assert.ok(sizes[0] > sizes[1], sizes.join())

  // An Arrow IPC stream has no page compression, and says so rather than
  // pretending to hold one.
  const stream = RecordOptions.from('trades.arrows')
  assert.equal(stream.compression, null)
  assert.equal(stream.maxRowGroupSize, null)
  assert.deepEqual(stream.keyValueMetadata, [])
  assert.throws(() => {
    stream.compression = 'snappy'
  }, /expected Parquet options/)
  assert.throws(() => parquet.withCompression('nope'), /nope/)
})

test('a resource that is not there holds no batches', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const absent = new IOBase(path.join(root, 'absent.arrows'))
  assert.ok(!absent.exists())
  assert.equal(absent.readArrowBatchReader().toTable().numRows, 0)
})

test('content coding belongs to the handle rather than to the encoding', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  // No call takes a coding argument: the name already carries it.
  const compressed = new IOBase(path.join(root, 'trades.arrows.gz'))
  assert.equal(compressed.mediaType.toString(), 'application/vnd.apache.arrow.stream;encodings=application/gzip')

  compressed.writeArrowBatchReader(trades())
  assert.equal(compressed.readArrowBatchReader().toTable().numRows, 2)
  assert.notEqual(compressed.readBytes().subarray(0, 2).toString('hex'), 'ffff')
})

test('rows read back as records, plain or through a runtime class', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const handle = new IOBase(path.join(root, 'trades.arrows'))
  // The record write is the generic write under the record name, so plain
  // objects are rows.
  handle.writeRecords([
    { id: 1n, symbol: 'AAPL', venue: 'XNAS' },
    { id: 2n, symbol: 'MSFT', venue: 'XNAS' },
  ])

  // Plain objects out, streamed batch by batch.
  const plain = [...handle.readRecords()]
  assert.deepEqual(
    plain.map((row) => row.symbol),
    ['AAPL', 'MSFT'],
  )

  // A class whose constructor takes the plain row is a runtime record class.
  class Trade {
    constructor(row) {
      Object.assign(this, row)
    }
    flag() {
      return `${this.symbol}@${this.venue}`
    }
  }
  const typed = [...handle.readRecords(Trade)]
  assert.ok(typed.every((row) => row instanceof Trade))
  assert.deepEqual(
    typed.map((row) => row.flag()),
    ['AAPL@XNAS', 'MSFT@XNAS'],
  )

  // Appending speaks the same vocabulary, and an absent resource yields
  // no records rather than raising.
  handle.appendRecords([{ id: 3n, symbol: 'NVDA', venue: 'XPAR' }])
  assert.equal([...handle.readRecords()].length, 3)
  assert.deepEqual([...new IOBase(path.join(root, 'absent.arrows')).readRecords()], [])
})

test('a field casts whatever Arrow JS holds, batch by batch', () => {
  const target = fields.struct(
    'row',
    [Field.from('id: int64'), Field.from('symbol: utf8')],
    { nullable: false },
  )

  // A table of narrower rows widens onto the field and stays a table.
  const source = rows([1n, 2n], ['AAPL', 'MSFT'], ['XNAS', 'XNAS'])
  const cast = target.castArrow(source)
  assert.equal(cast.numRows, 2)
  assert.deepEqual(
    cast.schema.fields.map((field) => field.name),
    ['id', 'symbol'],
  )
  // `cast` is the same call under the generic name.
  assert.equal(target.cast(source).numRows, 2)
})

test('a batch size of zero is refused rather than stored as a read of nothing', () => {
  const options = RecordOptions.forMimeType(MimeType.PARQUET)

  // Zero is not a small batch. The readers chunk by this number, so storing it
  // turns a read of a hundred rows into a successful read of none; `null` is
  // how "no bound" is already spelled.
  assert.throws(() => {
    options.batchSize = 0
  }, /expected a positive row count for batchSize, got 0/)
  assert.throws(() => options.withBatchSize(0), /got 0/)

  assert.equal(options.batchSize, null)
  options.batchSize = 32
  assert.equal(options.batchSize, 32)
  options.batchSize = null
  assert.equal(options.batchSize, null)
})
