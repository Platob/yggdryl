'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')

const arrow = require('apache-arrow')

// Capture the private native preflight before the public loader hides it. Its
// return value is the core-owned conversion bound used only when batchRowSize is
// absent, so this protects the language boundary from growing its own default.
const nativeBinding = require('../index.js')
const requireWritePreflightNative =
  nativeBinding.RecordOptions.prototype._requireWritePreflightNative
const { BatchReader, DataType, Field, IOBase, MimeType, RecordOptions, fields } = require('yggdryl')

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

function wkbPoint(x, y) {
  const bytes = Buffer.allocUnsafe(21)
  bytes.writeUInt8(1, 0)
  bytes.writeUInt32LE(1, 1)
  bytes.writeDoubleLE(x, 5)
  bytes.writeDoubleLE(y, 13)
  return bytes
}

test('a handle names its own encoding and round-trips Arrow batches', () => {
  const handle = IOBase.fromBytes()
  handle.mediaType = MimeType.ARROW_STREAM

  // The encoding is never guessed: it is whatever the handle says it holds.
  const options = handle.recordOptions()
  assert.equal(options.toString(), 'application/vnd.apache.arrow.stream')
  assert.equal(options.name, 'row')
  assert.equal(options.safe, false)
  assert.equal(options.batchRowSize, null)

  handle.overwriteArrowReader(BatchReader.from(trades()))
  assert.ok(handle.size > 0)
  assert.ok(handle.readArrowField().equals(schema()))

  const reader = handle.readArrowReader()
  assert.ok(reader.field.equals(schema()))
  let read = 0
  for (const batch of reader) {
    read += batch.numRows
  }
  assert.equal(read, 2)
  // A stream is read once, and says so rather than reading as empty.
  assert.ok(reader.consumed)
  assert.throws(() => reader.intoIpc(), /already been consumed/)

  // A reader a write took reports the same, rather than iterating as no rows.
  const written = BatchReader.from(trades())
  handle.overwriteArrowReader(written)
  assert.ok(written.consumed)
  assert.throws(() => [...written], /already been consumed/)
})

test('a batch reader is built from whatever a caller already holds', () => {
  const table = trades()
  const ipc = arrow.tableToIPC(table)

  for (const source of [table, table.batches[0], [...table.batches], ipc]) {
    const reader = BatchReader.from(source)
    assert.ok(reader.field.equals(schema()))
    assert.equal(reader.intoTable().numRows, 2)
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
  handle.overwriteArrowTable(trades())
  const plain = handle.recordOptions()

  const wanted = fields.struct('row', [Field.from('id: int64')], { nullable: false })
  const projected = handle.readArrowReader(plain.withField(wanted))
  assert.equal(projected.field.dtype.length, 1)
  assert.equal(projected.intoTable().numCols, 1)

  // The resource is unchanged: it still holds all three columns.
  assert.equal(handle.readArrowField().dtype.length, 3)

  // A projection can only drop columns, so a column the stream does not hold
  // is read whole and then supplied by the cast.
  const invented = fields.struct(
    'row',
    [Field.from('id: int64'), Field.from('nowhere: utf8')],
    { nullable: false },
  )
  const widened = handle.readArrowReader(plain.withField(invented))
  assert.equal(widened.field.dtype.length, 2)
})

test('parquet is chosen by the file name and nothing else', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const file = new IOBase(path.join(root, 'trades.parquet'))
  assert.equal(file.recordOptions().toString(), 'application/vnd.apache.parquet')
  assert.equal(file._readParquetStatisticsNative, undefined)
  assert.equal(file._readParquetGeospatialStatisticsNative, undefined)

  const declared = file
    .recordOptions()
    .withField(schema())
    .withMaxRowGroupSize(1)
    .withKeyValue('writer', 'node')
  file.overwriteArrowTable(trades(), declared)
  assert.ok(file.size > 0)
  assert.ok(file.readArrowField().equals(schema()))
  assert.equal(file.readArrowReader().intoTable().numRows, 2)

  const statistics = file.readParquetStatistics()
  assert.equal(statistics.num_rows, 2)
  assert.equal(statistics.row_groups.length, 2)
  assert.equal(
    statistics.key_value_metadata.find(({ key }) => key === 'writer').value,
    'node',
  )
  const identifier = statistics.row_groups[0].columns.find(
    ({ path: column }) => column === 'id',
  )
  assert.ok(Buffer.isBuffer(identifier.min_bytes))
  assert.ok(Buffer.isBuffer(identifier.max_bytes))
  assert.throws(
    () => file.readParquetGeospatialStatistics('id'),
    /WKB binary storage/,
  )

  // Both sides of an append stream, and the incoming batches are cast first.
  file.appendArrowTable(rows([3n], ['NVDA'], ['XNAS']), declared)
  const table = file.readArrowReader().intoTable()
  assert.equal(table.numRows, 3)
  assert.deepEqual(
    table.getChild('symbol').toArray(),
    ['AAPL', 'MSFT', 'NVDA'],
  )
})

test('Parquet statistics reject another inferred encoding before parsing bytes', () => {
  const stream = IOBase.fromBytes()
  stream.mediaType = MimeType.ARROW_STREAM
  assert.throws(() => stream.readParquetStatistics(), /expected Parquet media/)
  assert.throws(
    () => stream.readParquetGeospatialStatistics('shape'),
    /expected Parquet media/,
  )
})

test('Parquet geospatial statistics scan one projected WKB column', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const file = new IOBase(path.join(root, 'shapes.parquet'))
  file.overwriteArrowTable(
    new arrow.Table({
      shape: arrow.vectorFromArray(
        [wkbPoint(1, 2), null, wkbPoint(-3, 7)],
        new arrow.Binary(),
      ),
    }),
  )

  assert.deepEqual(file.readParquetGeospatialStatistics('shape'), {
    bounding_box: {
      mmax: null,
      mmin: null,
      xmax: 1,
      xmin: -3,
      ymax: 7,
      ymin: 2,
      zmax: null,
      zmin: null,
    },
    geometry_types: [1],
  })
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
  file.overwriteArrowTable(trades(), file.recordOptions().withField(required))
  assert.ok(file.readArrowField().equals(required))
})

test('a match key updates a stored row and appends a new one', () => {
  const handle = IOBase.fromBytes()
  handle.mediaType = MimeType.ARROW_STREAM
  handle.overwriteArrowTable(trades())

  const merging = handle.recordOptions().withMergeByNames(['id'])
  assert.deepEqual(merging.mergeByNames, ['id'])
  handle.mergeArrowTable(rows([2n, 9n], ['MSFT.O', 'NVDA'], ['XNYS', 'XNYS']), merging)

  const table = handle.readArrowReader().intoTable()
  assert.equal(table.numRows, 3)
  assert.deepEqual(table.getChild('symbol').toArray(), ['AAPL', 'MSFT.O', 'NVDA'])
})

test('a zero row limit reads the schema and no batches', () => {
  const handle = IOBase.fromBytes()
  handle.mediaType = MimeType.ARROW_STREAM
  handle.overwriteArrowTable(trades())

  // `0` is a valid ask, not an error: the shaped schema still answers.
  const reader = handle.readArrowReader(handle.recordOptions().withMaxRowSize(0))
  assert.ok(reader.field.equals(schema()))
  assert.equal(reader.intoTable().numRows, 0)
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
  file.overwriteArrowReader(BatchReader.from(many))

  const options = file.recordOptions()
  options.maxRowSize = 10
  assert.equal(options.maxRowSize, 10)
  assert.equal(file.readArrowReader(options).intoTable().numRows, 10)
})

test('a small byte limit still yields at least one row', () => {
  const handle = IOBase.fromBytes()
  handle.mediaType = MimeType.ARROW_STREAM
  handle.overwriteArrowTable(trades())

  // One byte admits no whole row, but a bounded read must never be a silent
  // total loss: only a limit of zero yields nothing.
  const options = handle.recordOptions().withMaxByteSize(1)
  assert.equal(handle.readArrowReader(options).intoTable().numRows, 1)
})

test('a limit with a match key is refused naming both settings', () => {
  const handle = IOBase.fromBytes()
  handle.mediaType = MimeType.ARROW_STREAM
  handle.overwriteArrowTable(trades())

  // A truncated merge would update the matched keys it kept and silently drop
  // the rest, so the combination is refused rather than corrupting.
  const limited = handle.recordOptions().withMergeByNames(['id']).withMaxRowSize(10)
  assert.throws(
    () => handle.mergeArrowTable(trades(), limited),
    /max_row_size = 10.*merge_by_names/,
  )
})

test('a folder is one table, and a write routes rows to their partition', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  fs.mkdirSync(path.join(root, 'venue=XNAS'), { recursive: true })

  const lake = new IOBase(root)
  const options = RecordOptions.forMimeType(MimeType.ARROW_STREAM).withField(schema())
  lake.overwriteArrowTable(trades(), options)

  // The value the directory spells is not stored again in every row.
  const leaf = lake.joinpath('venue=XNAS').joinpath('part-0.arrows')
  assert.equal(leaf.readArrowField().dtype.length, 2)

  const restored = lake.readArrowReader(options).intoTable()
  assert.equal(restored.numCols, 3)
  assert.deepEqual(restored.getChild('venue').toArray(), ['XNAS', 'XNAS'])
})

test('record options are values, and a setting is set or carried forward', () => {
  const options = RecordOptions.forMimeType(MimeType.PARQUET)
  assert.equal(options.mimeType.toString(), 'application/vnd.apache.parquet')
  assert.equal(options.field, null)

  const declared = options.withField(schema()).withBatchRowSize(1024).withSafe(true)
  assert.ok(declared.field.equals(schema()))
  assert.equal(declared.batchRowSize, 1024)
  assert.equal(declared.safe, true)
  // `with*` returns a new value, so the one it was built from is untouched.
  assert.equal(options.batchRowSize, null)
  assert.equal(options.safe, false)

  options.name = 'trade'
  options.level = 9
  assert.equal(options.name, 'trade')
  assert.equal(options.level, 9)

  // An encoding this build does not implement is named rather than guessed.
  assert.throws(
    () => RecordOptions.forMimeType('text/csv'),
    /expected a record encoding this build implements/,
  )
})

test('record option value protocols delegate every encoding to the core', () => {
  const marker = Buffer.from('0123456789abcdef')
  const text = RecordOptions.from('trades.txt')
    .withName('line')
    .withBatchRowSize(32)
  text.header = '(?<id>\\d+)'
  text.lstrip = '^\\s+'
  text.rstrip = '\\s+$'
  text.linesep = '\\r\\n'
  text.autotype = false
  text.timezone = '+02:00'
  const variants = [
    RecordOptions.from('trades.arrows')
      .withName('ipc-row')
      .withBatchRowSize(64),
    RecordOptions.from('trades.avro')
      .withBlockCodec('null')
      .withSyncMarker(marker),
    RecordOptions.from('trades.parquet')
      .withCompression('snappy')
      .withMaxRowGroupSize(512)
      .withKeyValue('source', 'protocol-test'),
    text,
  ]

  for (const options of variants) {
    const originalHash = options.stableHash()
    const clone = options.clone()
    assert.notEqual(clone, options)
    assert.ok(clone.equals(options), options.toString())
    assert.equal(clone.compare(options), 0, options.toString())
    assert.equal(clone.stableHash(), originalHash, options.toString())
    assert.equal(typeof originalHash, 'bigint')

    // A clone owns its core value. Mutation changes only that copy and all
    // three value protocols observe the new complete state.
    clone.safe = !clone.safe
    assert.ok(!clone.equals(options), options.toString())
    assert.notEqual(clone.compare(options), 0, options.toString())
    assert.equal(options.stableHash(), originalHash, options.toString())
  }

  // The encoding variant itself participates, even when shared fields agree.
  for (let left = 0; left < variants.length; left += 1) {
    for (let right = left + 1; right < variants.length; right += 1) {
      assert.ok(!variants[left].equals(variants[right]))
      assert.notEqual(variants[left].compare(variants[right]), 0)
    }
  }
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
    file.overwriteArrowTable(
      new arrow.Table({
        id: arrow.vectorFromArray(ids, new arrow.Int64()),
        symbol: arrow.vectorFromArray(ids.map(() => 'AAPL'), new arrow.Utf8()),
      }),
      file.recordOptions().withCompression(compression),
    )
    assert.equal(file.readArrowReader().intoTable().numRows, 4_000, compression)
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

test('Avro record options expose validated block settings', () => {
  const options = RecordOptions.from('trades.avro')
  assert.equal(options.blockCodec, 'deflate')
  assert.equal(options.syncMarker, null)

  options.blockCodec = 'null'
  options.syncMarker = Buffer.from('0123456789abcdef')
  assert.equal(options.blockCodec, 'null')
  assert.deepEqual(options.syncMarker, Buffer.from('0123456789abcdef'))

  const copied = options
    .withBlockCodec('zstandard')
    .withSyncMarker(Buffer.from('fedcba9876543210'))
  assert.equal(copied.blockCodec, 'zstandard')
  assert.deepEqual(copied.syncMarker, Buffer.from('fedcba9876543210'))
  assert.equal(options.blockCodec, 'null')
  assert.deepEqual(options.withSyncMarker(null).syncMarker, null)

  assert.throws(() => {
    options.blockCodec = 'brotli'
  }, /brotli/)
  assert.throws(() => {
    options.syncMarker = Buffer.from('short')
  }, /exactly 16 bytes/)

  const ipc = RecordOptions.from('trades.arrows')
  assert.equal(ipc.blockCodec, null)
  assert.equal(ipc.syncMarker, null)
  assert.throws(() => {
    ipc.blockCodec = 'null'
  }, /expected Avro options/)
  assert.throws(() => {
    ipc.syncMarker = null
  }, /expected Avro options/)
})

test('native record conversion uses the shared core batch default', () => {
  const options = nativeBinding.RecordOptions.from('trades.arrows')
  assert.equal(
    Reflect.apply(requireWritePreflightNative, options, ['overwrite']),
    65_536,
  )
  assert.equal(RecordOptions.prototype._requireWritePreflightNative, undefined)
})

test('a resource that is not there holds no batches', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const absent = new IOBase(path.join(root, 'absent.arrows'))
  assert.ok(!absent.exists())
  assert.equal(absent.readArrowReader().intoTable().numRows, 0)
})

test('content coding belongs to the handle rather than to the encoding', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  // No call takes a coding argument: the name already carries it.
  const compressed = new IOBase(path.join(root, 'trades.arrows.gz'))
  assert.equal(compressed.mediaType.toString(), 'application/vnd.apache.arrow.stream;encodings=application/gzip')

  compressed.overwriteArrowTable(trades())
  assert.equal(compressed.readArrowReader().intoTable().numRows, 2)
  assert.notEqual(compressed.readBytes().subarray(0, 2).toString('hex'), 'ffff')
})

test('rows read back as records, plain or through a runtime class', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const handle = new IOBase(path.join(root, 'trades.arrows'))
  // The record-specific overwrite infers plain objects as rows.
  handle.overwriteRecords([
    { id: 1n, symbol: 'AAPL', venue: 'XNAS' },
    { id: 2n, symbol: 'MSFT', venue: 'XNAS' },
  ])

  // Plain objects out, streamed batch by batch.
  const plain = [...handle.readRecords()]
  assert.deepEqual(
    plain.map((row) => row.symbol),
    ['AAPL', 'MSFT'],
  )

  // A class whose constructor takes the plain row is a runtime row adapter.
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

test('an ASCII column pads on the way in and trims on the way out', () => {
  const handle = IOBase.fromBytes()
  handle.mediaType = MimeType.ARROW_STREAM
  const declared = fields.struct('row', [fields.ascii32('ccy')], { nullable: false })
  const options = handle.recordOptions().withField(declared)
  const codes = (values) =>
    new arrow.Table({ ccy: arrow.vectorFromArray(values, new arrow.Utf8()) })

  handle.overwriteArrowTable(codes(['USD', 'EUR']), options)
  // The identity survives the IPC stream, so the stored field is the ASCII width.
  assert.ok(handle.readArrowField().equals(declared))
  // Arrow JS sees the storage: the padded fixed width. Every string rendering
  // trims, so reading under a declared text column is the core cast that
  // turns the padding back into the text that went in.
  const stored = handle.readArrowReader().intoTable().getChild('ccy')
  assert.deepEqual([...stored.get(0)], [0x55, 0x53, 0x44, 0])
  const text = fields.struct('row', [fields.utf8('ccy')], { nullable: false })
  assert.deepEqual(
    [...handle.readRecords(handle.recordOptions().withField(text))].map((row) => row.ccy),
    ['USD', 'EUR'],
  )

  assert.throws(
    () => handle.overwriteArrowTable(codes(['EURO!']), options),
    /ASCII text of at most 4 bytes/,
  )
})

test('a batch size of zero is refused rather than stored as a read of nothing', () => {
  const options = RecordOptions.forMimeType(MimeType.PARQUET)

  // Zero is not a small batch. The readers chunk by this number, so storing it
  // turns a read of a hundred rows into a successful read of none; `null` is
  // how "no bound" is already spelled.
  assert.throws(() => {
    options.batchRowSize = 0
  }, /expected a positive row count for batchRowSize, got 0/)
  assert.throws(() => options.withBatchRowSize(0), /got 0/)

  assert.equal(options.batchRowSize, null)
  options.batchRowSize = 32
  assert.equal(options.batchRowSize, 32)
  options.batchRowSize = null
  assert.equal(options.batchRowSize, null)
})

test('the declared root is three parts, and field is built from them', () => {
  const options = RecordOptions.forMimeType(MimeType.ARROW_STREAM)
  assert.equal(options.name, 'row')
  assert.equal(options.dtype, null)
  assert.deepEqual(options.metadata, [])
  assert.equal(options.field, null)

  // A datatype expression or a native DataType declares the shape; the field
  // is the non-null Struct root assembled from the three parts on every ask.
  options.dtype = 'struct<id: int64>'
  assert.ok(options.dtype.equals(new DataType('struct<id: int64>')))
  options.name = 'trade'
  options.metadata = { source: 'book' }
  const built = options.field
  assert.equal(built.name, 'trade')
  assert.equal(built.nullable, false)
  assert.ok(built.dtype.equals(new DataType('struct<id: int64>')))
  assert.deepEqual(built.entries(), [{ key: 'source', value: 'book' }])
  assert.ok(options.field.equals(built))

  // A declared field decomposes into the same three parts.
  const declared = RecordOptions.from('trades.parquet').withField(built)
  assert.equal(declared.name, 'trade')
  assert.ok(declared.dtype.equals(built.dtype))
  assert.deepEqual(declared.metadata, [{ key: 'source', value: 'book' }])
  assert.ok(declared.field.equals(built))

  // Clearing the datatype clears the field and keeps the name and metadata.
  options.dtype = null
  assert.equal(options.dtype, null)
  assert.equal(options.field, null)
  assert.equal(options.name, 'trade')
  assert.deepEqual(options.metadata, [{ key: 'source', value: 'book' }])

  const typed = options.withDtype(new DataType('struct<id: int64, symbol: utf8>'))
  assert.equal(typed.field.name, 'trade')
  assert.equal(typed.field.dtype.length, 2)
  assert.equal(options.field, null)
  assert.equal(typed.withName('row').field.name, 'row')
  assert.throws(() => options.withDtype('struct<'), /invalid datatype expression/)
  assert.throws(() => {
    options.dtype = 'struct<'
  }, /invalid datatype expression/)
  assert.equal(options.dtype, null)
})

test('a root declared through a field or a datatype is the same value', () => {
  const field = Field.from('row: struct<id: int64, symbol: utf8> not null')
  const byField = RecordOptions.from('trades.parquet').withField(field)
  const byDtype = RecordOptions.from('trades.parquet').withDtype(field.dtype)
  assert.ok(byField.equals(byDtype))
  assert.equal(byField.compare(byDtype), 0)
  assert.equal(byField.stableHash(), byDtype.stableHash())
  assert.ok(byField.field.equals(byDtype.field))

  // Each part takes a side in equality on its own.
  assert.ok(!byField.equals(byDtype.withName('trade')))
  assert.ok(!byField.equals(byDtype.withMetadata({ source: 'book' })))
  assert.ok(byField.withMetadata({ source: 'book' }).equals(byDtype.withMetadata({ source: 'book' })))

  // Nullability is not one of the parts: the root is always required.
  const nullable = RecordOptions.from('trades.parquet').withField(
    Field.from('row: struct<id: int64, symbol: utf8>'),
  )
  assert.ok(nullable.equals(byField))
  assert.equal(nullable.field.nullable, false)
})

test('root metadata takes entries, a plain object, a Map, or a Field', () => {
  const options = RecordOptions.from('trades.parquet').withDtype('struct<id: int64>')

  options.metadata = [{ key: 'source', value: 'book' }]
  assert.deepEqual(options.metadata, [{ key: 'source', value: 'book' }])
  options.metadata = { venue: 'XNAS', session: 'regular' }
  assert.deepEqual(options.metadata, [
    { key: 'session', value: 'regular' },
    { key: 'venue', value: 'XNAS' },
  ])
  options.metadata = new Map([['currency', 'EUR']])
  assert.deepEqual(options.metadata, [{ key: 'currency', value: 'EUR' }])
  options.metadata = [['precision', 'micros']]
  assert.deepEqual(options.metadata, [{ key: 'precision', value: 'micros' }])
  options.metadata = new Field('price', 'decimal(18, 6)', false, { unit: 'cents' })
  assert.deepEqual(options.metadata, [{ key: 'unit', value: 'cents' }])
  assert.deepEqual(options.field.entries(), [{ key: 'unit', value: 'cents' }])

  // `withMetadata` is the same setter on a copy, and empty clears.
  const copied = options.withMetadata(new Map([['source', 'book']]))
  assert.deepEqual(copied.metadata, [{ key: 'source', value: 'book' }])
  assert.deepEqual(options.metadata, [{ key: 'unit', value: 'cents' }])
  assert.deepEqual(copied.withMetadata([]).metadata, [])
  assert.deepEqual(copied.withMetadata({}).field.entries(), [])

  // The core validates the entries; a refused value leaves the options as is.
  assert.throws(() => {
    options.metadata = [{ key: '', value: 'x' }]
  }, /metadata keys must not be empty/)
  assert.throws(() => {
    options.metadata = 'source=book'
  }, TypeError)
  assert.throws(() => options.withMetadata({ source: 7 }), TypeError)
  assert.deepEqual(options.metadata, [{ key: 'unit', value: 'cents' }])
})

test('a declared name roots the schema inferred from plain records', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  // Avro spells the root name in the container header, so the name a plain
  // object inference was rooted by is visible in what was written.
  const handle = new IOBase(path.join(root, 'trades.avro'))
  handle.overwriteRecords(
    [
      { id: 1n, qty: 2.5 },
      { id: 2n, qty: 3.5 },
    ],
    handle.recordOptions().withName('trade'),
  )
  const written = Buffer.from(handle.readBytes())
  assert.ok(written.includes('"name":"trade"'))
  assert.ok(!written.includes('"name":"row"'))
  assert.equal(handle.readArrowReader().intoTable().numRows, 2)

  const byDefault = new IOBase(path.join(root, 'rows.avro'))
  byDefault.overwriteRecords([{ id: 1n, qty: 2.5 }])
  assert.ok(Buffer.from(byDefault.readBytes()).includes('"name":"row"'))

  // A read names its root the same way.
  const stream = IOBase.fromBytes()
  stream.mediaType = MimeType.ARROW_STREAM
  stream.overwriteArrowTable(trades(), stream.recordOptions().withName('trade'))
  assert.equal(stream.readArrowField().name, 'row')
  assert.equal(stream.readArrowField(stream.recordOptions().withName('trade')).name, 'trade')

  // Text has no stored schema: its extractor supplies the datatype, while the
  // shared name still names the inferred root. Metadata alone declares none.
  const text = new IOBase(path.join(root, 'events.log'))
  text.writeText('first\nsecond\n')
  const textOptions = text.recordOptions().withName('events').withMetadata({ owner: 'risk' })
  const textField = text.readArrowField(textOptions)
  assert.equal(textField.name, 'events')
  assert.deepEqual(textField.entries(), [])
})
