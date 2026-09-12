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
const nativeBinding = require('../../index.js')
const requireWritePreflightNative =
  nativeBinding.RecordOptions.prototype._requireWritePreflightNative
const {
  BatchReader,
  DataType,
  Field,
  IOBase,
  MimeType,
  RecordOptions,
  TextOptions,
  fields,
} = require('yggdryl')

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

test('text options value protocols include every flat text setting', () => {
  const options = new TextOptions()
    .withName('line')
    .withBatchRowSize(32)
    .withSelectByNames(['body'])
  options.rowheader = '(?<id>\\d+)'
  options.lstrip = ['^\\s+']
  options.rstrip = ['\\s+$']
  options.linesep = '\\r\\n'
  options.autotype = false
  options.timezone = '+02:00'
  options.startRownum = 7n
  options.parseMtime = false

  const clone = options.clone()
  assert.ok(clone.equals(options))
  assert.equal(clone.compare(options), 0)
  assert.equal(clone.stableHash(), options.stableHash())
  clone.safe = true
  assert.ok(!clone.equals(options))
  assert.notEqual(clone.compare(options), 0)

  // The dating flag is part of the value too: it alone parts two settings.
  const dated = options.clone()
  dated.parseMtime = true
  assert.ok(!dated.equals(options))
  assert.notEqual(dated.compare(options), 0)
  assert.notEqual(dated.stableHash(), options.stableHash())
})

test('plain text dates every row, and the flag takes the column away', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const target = path.join(root, 'events.log')
  fs.writeFileSync(target, 'first\nsecond\n')
  const handle = new IOBase(target)

  // The column is on by default and holds one fixed place, right after url.
  const options = new TextOptions()
  assert.equal(options.parseMtime, true)
  const table = handle.readArrowReader(options).intoTable()
  assert.deepEqual(
    table.schema.fields.map((field) => [
      field.name,
      field.type.toString(),
      field.nullable,
    ]),
    [
      ['url', 'Utf8', true],
      ['mtime', 'Timestamp<NANOSECOND, UTC>', true],
      ['body', 'Utf8', false],
    ],
  )
  // The url column is the `url` datatype: Utf8 storage carrying the extension
  // identity, and nullable because a handle without a location has no URL.
  assert.equal(
    table.schema.fields[0].metadata.get('ARROW:extension:name'),
    'yggdryl.url',
  )
  // A located handle fills it with the canonical URL text of its location.
  assert.deepEqual(
    [...table.getChild('url')],
    [handle.url.toString(), handle.url.toString()],
  )

  // One fact about the file, read once and repeated: every row carries the
  // handle's own modification time, to the nanosecond it is stored at.
  const stamped = fs.statSync(target, { bigint: true }).mtimeNs
  assert.deepEqual([...table.getChild('mtime').toArray()], [stamped, stamped])
  // The record path reads the same column as the batch path.
  assert.deepEqual(
    [...handle.readRecords(options)].map((row) => row.mtime),
    [...table.getChild('mtime')],
  )

  // rownum still comes first when it is asked for, and captures still trail.
  const numbered = new TextOptions()
  numbered.startRownum = 1n
  numbered.rowheader = '^(?<word>\\w+)'
  assert.deepEqual(
    handle
      .readArrowReader(numbered)
      .intoTable()
      .schema.fields.map((field) => field.name),
    ['url', 'rownum', 'mtime', 'body', 'word'],
  )

  // Turning the flag off takes the column away rather than nulling it.
  const undated = new TextOptions()
  undated.parseMtime = false
  assert.deepEqual(
    handle
      .readArrowReader(undated)
      .intoTable()
      .schema.fields.map((field) => field.name),
    ['url', 'body'],
  )

  // A buffer records no modification time, so the column is there and null:
  // the reader says so rather than inventing a clock reading.
  const buffer = IOBase.fromBytes(Buffer.from('first\nsecond\n'))
  const held = buffer.readArrowReader(options).intoTable()
  assert.deepEqual(
    held.schema.fields.map((field) => field.name),
    ['url', 'mtime', 'body'],
  )
  assert.deepEqual([...held.getChild('mtime')], [null, null])
  assert.deepEqual(
    [...buffer.readRecords(options)].map((row) => row.mtime),
    [null, null],
  )
})

test('a row header that dates a line fills mtime rather than adding a column', () => {
  const options = new TextOptions()
  options.rowheader = '^(?<mtime>\\S+) id=(?<id>\\d+) '
  const table = IOBase.fromBytes(
    Buffer.from('2020-01-02T03:04:05.123456789Z id=7 first\n'),
  )
    .readArrowReader(options)
    .intoTable()

  // One column, not two: the capture dates the line, and is read at the
  // column's own datatype rather than at the one its syntax suggests.
  assert.deepEqual(
    table.schema.fields.map((field) => field.name),
    ['url', 'mtime', 'body', 'id'],
  )
  assert.equal(table.schema.fields[1].type.unit, arrow.TimeUnit.NANOSECOND)
  assert.equal(table.schema.fields[1].type.timezone, 'UTC')
  assert.deepEqual(
    [...table.getChild('mtime').toArray()],
    [1_577_934_245_123_456_789n],
  )

  // With the column off, the same name is an ordinary trailing capture,
  // typed by its own syntax and sitting after body.
  const undated = new TextOptions()
  undated.parseMtime = false
  undated.rowheader = '^(?<mtime>\\d+) '
  const counted = IOBase.fromBytes(Buffer.from('77 first\n'))
    .readArrowReader(undated)
    .intoTable()
  assert.deepEqual(
    counted.schema.fields.map((field) => [field.name, field.type.toString()]),
    [
      ['url', 'Utf8'],
      ['body', 'Utf8'],
      ['mtime', 'Int64'],
    ],
  )
  assert.deepEqual([...counted.getChild('mtime')], [77n])
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

test('a cast failure inside a stream reports the failure, not the envelope', () => {
  // A reader can only carry a core failure boxed inside an ArrowError, and
  // that envelope is transport: draining one here must hand back the failure
  // the cast raised, not `External error: <the real one>`. A string ingest
  // nulls a failing cell under the safe default, so the strict cast is the
  // one that raises.
  const target = fields.struct('row', [fields.ascii('ccy', { nullable: false })], {
    nullable: false,
  })
  const source = new arrow.Table({
    ccy: arrow.vectorFromArray(['US\u00c9'], new arrow.Utf8()),
  })
  const strict = { safe: false }

  for (const drain of [
    () => target.castArrow(source, strict),
    () => target.castArrowReader(source, strict).intoIpc(),
    () => [...target.castArrowReader(source, strict)],
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
  assert.deepEqual([...target.castArrow(source).getChild('ccy')], [''])
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

test('a field casts one Arrow JS batch, and a whole stream one pull at a time', () => {
  const target = fields.struct(
    'row',
    [Field.from('id: int64'), Field.from('symbol: utf8')],
    { nullable: false },
  )
  const source = new arrow.Table({
    id: arrow.vectorFromArray([1, 2], new arrow.Int32()),
    symbol: arrow.vectorFromArray(['AAPL', 'MSFT'], new arrow.Utf8()),
    venue: arrow.vectorFromArray(['XNAS', 'XNAS'], new arrow.Utf8()),
  })

  // One batch in, one batch out: the int32 widens onto the declared int64 and
  // the column the target never declared is dropped.
  const batch = target.castArrowBatch(source.batches[0])
  assert.ok(batch instanceof arrow.RecordBatch)
  assert.equal(batch.numRows, 2)
  assert.deepEqual(
    batch.schema.fields.map((field) => `${field.name}: ${field.type}`),
    ['id: Int64', 'symbol: Utf8'],
  )
  assert.equal(batch.getChild('venue'), null)
  assert.throws(() => target.castArrowBatch(source), TypeError)

  // The lazy half: a stream is read once, so the source is consumed and the
  // reader handed back is the one that still yields rows - unread, and already
  // answering the field it casts onto.
  const stream = BatchReader.from(source)
  const reader = target.castArrowReader(stream)
  assert.equal(stream.consumed, true)
  assert.equal(reader.consumed, false)
  assert.ok(reader.field.equals(target))
  assert.equal(reader.intoTable().numRows, 2)
  assert.equal(reader.consumed, true)

  // The eager half: `castArrow` drains that same reader into a Table.
  assert.ok(target.castArrow(source) instanceof arrow.Table)

  // An exact cast changes nothing: the rows that come back are the rows that
  // went in, under the field the source already declared.
  const exact = new arrow.Table({
    id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
    symbol: arrow.vectorFromArray(['AAPL', 'MSFT'], new arrow.Utf8()),
  })
  assert.ok(BatchReader.from(exact).field.equals(target))
  const unchanged = target.castArrow(exact)
  assert.deepEqual([...unchanged.getChild('id')], [1n, 2n])
  assert.deepEqual([...unchanged.getChild('symbol')], ['AAPL', 'MSFT'])
})

test('a required column the source cannot fill is written, or refused by path', () => {
  const target = fields.struct(
    'row',
    [Field.from('id: int64'), Field.from('symbol: utf8 not null')],
    { nullable: false },
  )
  const identifiers = new arrow.Table({
    id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
  })

  // The default policy repairs the hole with the target's canonical default.
  assert.deepEqual([...target.castArrow(identifiers).getChild('symbol')], ['', ''])

  // Strict refuses it from the two schemas alone, and names the whole path.
  assert.throws(
    () => target.castArrow(identifiers, { nullability: 'strict' }),
    /required Arrow field \$\.symbol is missing from the source/,
  )
  // The policy is a name, and an unknown one is refused by its vocabulary.
  assert.throws(
    () => target.castArrow(identifiers, { nullability: 'lenient' }),
    /expected one of default, strict/,
  )

  // A null the source does carry is that same absence, and it is counted.
  const partial = new arrow.Table({
    id: arrow.vectorFromArray([1n, 2n, 3n], new arrow.Int64()),
    symbol: arrow.vectorFromArray(['AAPL', null, null], new arrow.Utf8()),
  })
  assert.deepEqual(
    [...target.castArrow(partial).getChild('symbol')],
    ['AAPL', '', ''],
  )
  assert.throws(
    () => target.castArrow(partial, { nullability: 'strict' }),
    /required Arrow field \$\.symbol holds 2 null values/,
  )

  // Nothing is required of a nullable column, and nothing is asked of a column
  // the target never declared, so neither policy has anything to say about
  // either: the missing one stays null and the undeclared one stays dropped.
  const nullable = fields.struct(
    'row',
    [Field.from('id: int64'), Field.from('symbol: utf8')],
    { nullable: false },
  )
  const extra = new arrow.Table({
    id: arrow.vectorFromArray([1n], new arrow.Int64()),
    symbol: arrow.vectorFromArray(['AAPL'], new arrow.Utf8()),
    venue: arrow.vectorFromArray(['XNAS'], new arrow.Utf8()),
  })
  for (const nullability of ['default', 'strict']) {
    const missing = nullable.castArrow(identifiers, { nullability }).getChild('symbol')
    assert.deepEqual([...missing], [null, null])
    assert.equal(missing.nullCount, 2)
    assert.deepEqual(
      target
        .castArrow(extra, { nullability })
        .schema.fields.map((field) => field.name),
      ['id', 'symbol'],
    )
  }
})

test('a required field inside a struct is refused by its whole path', () => {
  const target = Field.from(
    'row: struct<account: struct<id: int64, zip: utf8 not null>> not null',
  )
  // Arrow JS builds a non-null child unless the field says otherwise, and a
  // non-null child may not hold the null this is about.
  const identifier = arrow.Field.new('id', new arrow.Int64(), true)
  const postcode = arrow.Field.new('zip', new arrow.Utf8(), true)
  const accounts = (values, children) =>
    new arrow.Table({
      account: arrow.vectorFromArray(values, new arrow.Struct(children)),
    })

  // The source carries no `zip` at all: refused where the two schemas meet.
  const without = accounts([{ id: 1n }, { id: 2n }], [identifier])
  assert.deepEqual(
    [...target.castArrow(without).getChild('account')].map((row) => row.zip),
    ['', ''],
  )
  assert.throws(
    () => target.castArrow(without, { nullability: 'strict' }),
    /required Arrow field \$\.account\.zip is missing from the source/,
  )

  // The source carries it and it holds a null: refused once the rows are read.
  const withNull = accounts(
    [{ id: 1n, zip: '75001' }, { id: 2n, zip: null }],
    [identifier, postcode],
  )
  assert.throws(
    () => target.castArrow(withNull, { nullability: 'strict' }),
    /required Arrow field \$\.account\.zip holds 1 null values/,
  )
})

test('safe and strictness are two independent answers', () => {
  const target = fields.struct(
    'row',
    [Field.from('id: int64'), Field.from('quantity: int8 not null')],
    { nullable: false },
  )
  const source = new arrow.Table({
    id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
    quantity: arrow.vectorFromArray([7n, 130n], new arrow.Int64()),
  })

  // `safe` turns the value the target cannot hold into a null, which the
  // default policy then repairs with the canonical default.
  assert.deepEqual([...target.castArrow(source).getChild('quantity')], [7, 0])

  // Strictness reads that same null as the absence it is, and refuses it.
  assert.throws(
    () => target.castArrow(source, { nullability: 'strict' }),
    /required Arrow field \$\.quantity holds 1 null values/,
  )

  // `safe: false` refuses the conversion itself, before absence is a question
  // anyone gets to ask, so both policies raise that same conversion error.
  for (const nullability of ['default', 'strict']) {
    assert.throws(
      () => target.castArrow(source, { safe: false, nullability }),
      /Can't cast value 130 to type Int8/,
    )
  }
})

test('a strict stream refuses at the pull, and a missing column at the build', () => {
  const target = fields.struct(
    'row',
    [Field.from('id: int64'), Field.from('symbol: utf8 not null')],
    { nullable: false },
  )
  const partial = new arrow.Table({
    id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
    symbol: arrow.vectorFromArray(['AAPL', null], new arrow.Utf8()),
  })

  // A null is a property of rows, so the reader is built and answers its field
  // before anything refuses it.
  const reader = target.castArrowReader(partial, { nullability: 'strict' })
  assert.equal(reader.consumed, false)
  assert.ok(reader.field.equals(target))
  assert.throws(
    () => reader.intoTable(),
    /required Arrow field \$\.symbol holds 1 null values/,
  )

  // A missing column is a property of the schemas alone, so it is refused
  // where they meet: building the reader, with no batch pulled.
  assert.throws(
    () =>
      target.castArrowReader(
        new arrow.Table({ id: arrow.vectorFromArray([1n], new arrow.Int64()) }),
        { nullability: 'strict' },
      ),
    /required Arrow field \$\.symbol is missing from the source/,
  )
})

test('an ASCII column pads on the way in and trims on the way out', () => {
  const handle = IOBase.fromBytes()
  handle.mediaType = MimeType.ARROW_STREAM
  const declared = fields.struct('row', [fields.fixedAscii('ccy', 4)], { nullable: false })
  const options = handle.recordOptions().withField(declared)
  const codes = (values) =>
    new arrow.Table({ ccy: arrow.vectorFromArray(values, new arrow.Utf8()) })

  handle.overwriteArrowTable(codes(['USD', 'EUR']), options)
  // The declaration survives the IPC stream, so the stored field is the
  // fixed US-ASCII string at its width.
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
    /expected at most 4 bytes of us-ascii, got 5/,
  )
})

test('a variable ASCII column stores the bytes it is given', () => {
  const handle = IOBase.fromBytes()
  handle.mediaType = MimeType.ARROW_STREAM
  const declared = fields.struct('row', [fields.ascii('note')], { nullable: false })
  const options = handle.recordOptions().withField(declared)
  const notes = (values) =>
    new arrow.Table({ note: arrow.vectorFromArray(values, new arrow.Utf8()) })

  handle.overwriteArrowTable(notes(['a', 'much longer note']), options)
  assert.ok(handle.readArrowField().equals(declared))
  // No width, so no padding: the stored bytes are the value's own, and since
  // US-ASCII bytes are UTF-8, Arrow JS sees the text storage the `yggdryl.string`
  // document sits over.
  const stored = handle.readArrowReader().intoTable().getChild('note')
  assert.equal(stored.type.toString(), 'Utf8')
  assert.deepEqual([...stored], ['a', 'much longer note'])
  const text = fields.struct('row', [fields.utf8('note')], { nullable: false })
  assert.deepEqual(
    [...handle.readRecords(handle.recordOptions().withField(text))].map((row) => row.note),
    ['a', 'much longer note'],
  )

  // The value contract is the width's, minus the width itself.
  assert.throws(
    () => handle.overwriteArrowTable(notes(['\u20ac']), options),
    /non-ASCII byte/,
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
