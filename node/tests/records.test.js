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

// The eighteen event columns every line batch opens with: the line as the
// event it is, the same eighteen a FIX row parsed out of it opens with.
const EVENT_COLUMNS = [
  'currunix', 'creaunix', 'execunix', 'recdunix', 'refrecdunix',
  'exprtime', 'prevunix', 'snapunix',
  'curruuid', 'crossuuid', 'crosscode', 'currhashcode', 'crosshashcode',
  'prevuuid', 'seqnum', 'srcuuids', 'identifiers', 'state',
]

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

  const merging = handle.recordOptions().withMergeBy(['id'])
  assert.deepEqual(merging.mergeBy.names, ['id'])
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
  const limited = handle.recordOptions().withMergeBy(['id']).withMaxRowSize(10)
  assert.throws(
    () => handle.mergeArrowTable(trades(), limited),
    /max_row_size = 10.*merge_by `id`/,
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
    .withSelect(['body'])
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
    table.schema.fields.slice(0, EVENT_COLUMNS.length).map((field) => field.name),
    EVENT_COLUMNS,
  )
  assert.deepEqual(
    table.schema.fields.slice(EVENT_COLUMNS.length).map((field) => [
      field.name,
      field.type.toString(),
      field.nullable,
    ]),
    [['body', 'Utf8', false]],
  )
  // The object a line came from is its chain, so a located handle fills
  // `crosscode` with the canonical URL text of its location.
  assert.deepEqual(
    [...table.getChild('crosscode')],
    [handle.url.toString(), handle.url.toString()],
  )

  // One fact about the file, read once and repeated: every row is dated by
  // the handle's own modification time, to the nanosecond it is stored at.
  const stamped = fs.statSync(target, { bigint: true }).mtimeNs
  assert.deepEqual([...table.getChild('currunix').toArray()], [stamped, stamped])
  // The record path reads the same column as the batch path.
  assert.deepEqual(
    [...handle.readRecords(options)].map((row) => row.currunix),
    [...table.getChild('currunix')],
  )

  // Captures trail the line's own columns, and the row number needs none of
  // its own: the event states it.
  const numbered = new TextOptions()
  numbered.startRownum = 1n
  numbered.rowheader = '^(?<word>\\w+)'
  assert.deepEqual(
    handle
      .readArrowReader(numbered)
      .intoTable()
      .schema.fields.map((field) => field.name),
    [...EVENT_COLUMNS, 'body', 'word'],
  )
  assert.deepEqual(
    [...handle.readArrowReader(numbered).intoTable().getChild('seqnum')],
    [1n, 2n],
  )

  // A buffer records no modification time, so nothing dates its lines and
  // the instant reads as the epoch rather than an invented clock reading.
  const buffer = IOBase.fromBytes(Buffer.from('first\nsecond\n'))
  const held = buffer.readArrowReader(options).intoTable()
  assert.deepEqual(
    held.schema.fields.map((field) => field.name),
    [...EVENT_COLUMNS, 'body'],
  )
  assert.deepEqual([...held.getChild('currunix').toArray()], [0n, 0n])
})

test('a row header that dates a line fills currunix rather than adding a column', () => {
  const options = new TextOptions()
  options.rowheader = '^(?<mtime>\\S+) id=(?<id>\\d+) '
  const table = IOBase.fromBytes(
    Buffer.from('2020-01-02T03:04:05.123456789Z id=7 first\n'),
  )
    .readArrowReader(options)
    .intoTable()

  // No column of its own: the capture dates the line, and the instant it
  // states is the event's, read at that clock rather than at the datatype
  // its syntax suggests.
  assert.deepEqual(
    table.schema.fields.map((field) => field.name),
    [...EVENT_COLUMNS, 'body', 'id'],
  )
  const currunix = table.schema.fields.find((field) => field.name === 'currunix')
  assert.ok(currunix)
  assert.equal(currunix.type.unit, arrow.TimeUnit.NANOSECOND)
  assert.equal(currunix.type.timezone, 'UTC')
  assert.deepEqual(
    [...table.getChild('currunix').toArray()],
    [1_577_934_245_123_456_789n],
  )

  // With the flag off, the same name is an ordinary trailing capture,
  // typed by its own syntax and sitting after body.
  const undated = new TextOptions()
  undated.parseMtime = false
  undated.rowheader = '^(?<mtime>\\d+) '
  const counted = IOBase.fromBytes(Buffer.from('77 first\n'))
    .readArrowReader(undated)
    .intoTable()
  assert.deepEqual(
    counted.schema.fields
      .slice(EVENT_COLUMNS.length)
      .map((field) => [field.name, field.type.toString()]),
    [
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

test('the declared root is one section: the field the plan creates', () => {
  const options = RecordOptions.forMimeType(MimeType.ARROW_STREAM)
  assert.equal(options.name, 'row')
  assert.equal(options.field, null)
  assert.ok(options.select.isAll)
  assert.ok(options.filter.isAlwaysTrue)
  assert.ok(options.mergeBy.isAll)

  // A field declares the shape whole; the stored root is the required Struct
  // it names, with its metadata.
  const declared = new Field('trade', 'struct<id: int64>', true, { source: 'book' })
  options.field = declared
  const built = options.field
  assert.equal(built.name, 'trade')
  assert.equal(built.nullable, false)
  assert.ok(built.dtype.equals(new DataType('struct<id: int64>')))
  assert.deepEqual(built.entries(), [{ key: 'source', value: 'book' }])
  assert.equal(options.name, 'trade')

  // The field is the plan's `create` section, spelled as one.
  assert.ok(options.plan.toString().startsWith('create trade ('))
  assert.ok(options.plan.field().equals(options.field))

  // The name is one part of the field, so renaming renames it. The default
  // root name is not a location, so the section prints without one.
  options.name = 'row'
  assert.equal(options.field.name, 'row')
  assert.equal(declared.name, 'trade')
  assert.ok(options.plan.toString().startsWith('create ('))
  assert.ok(options.plan.field().equals(options.field))

  // Clearing the field leaves the name alone.
  options.field = null
  assert.equal(options.field, null)
  assert.equal(options.name, 'row')
  const typed = options.withField(Field.from('row: struct<id: int64, symbol: utf8> not null'))
  assert.equal(typed.field.dtype.length, 2)
  assert.equal(options.field, null)
  assert.equal(typed.withName('trade').field.name, 'trade')
})

test('a root declared through a field or a plan is the same value', () => {
  const field = Field.from('row: struct<id: int64, symbol: utf8> not null')
  const byField = RecordOptions.from('trades.parquet').withField(field)
  const byPlan = RecordOptions.from('trades.parquet').withPlan(field)
  assert.ok(byField.equals(byPlan))
  assert.equal(byField.compare(byPlan), 0)
  assert.equal(byField.stableHash(), byPlan.stableHash())
  assert.ok(byField.field.equals(byPlan.field))

  // Each section takes a side in equality on its own.
  assert.ok(!byField.equals(byPlan.withName('trade')))
  assert.ok(!byField.equals(byPlan.withSelect(['id'])))
  assert.ok(byField.withFilter('id > 1').equals(byPlan.withFilter('id > 1')))

  // Nullability is not one of the parts: the root is always required.
  const nullable = RecordOptions.from('trades.parquet').withField(
    Field.from('row: struct<id: int64, symbol: utf8>'),
  )
  assert.ok(nullable.equals(byField))
  assert.equal(nullable.field.nullable, false)
})

test('the sections are one plan, and a plan splits back into them', () => {
  const options = RecordOptions.from('trades.parquet')
  options.filter = "venue = 'XNAS' and id > 5"
  options.select = 'id, symbol'
  options.mergeBy = ['id']
  options.maxRowSize = 10
  options.field = Field.from('row: struct<id: int64, symbol: utf8, venue: utf8> not null')

  // The equalities the filter pins are what prune a listing before anything
  // is opened, spelled as partition paths spell them.
  assert.deepEqual(options.partitionPairs(), [['venue', 'XNAS']])
  const plan = options.plan
  assert.equal(plan.verb, 'upsert into')
  assert.ok(plan.selector.equals('id, symbol'))
  assert.equal(plan.limit, 10)
  assert.ok(plan.filter.equals("venue = 'XNAS' and id > 5"))

  const fresh = RecordOptions.from('trades.parquet')
  fresh.plan = plan.toString()
  assert.ok(fresh.equals(options))
  assert.ok(fresh.field.equals(options.field))
  assert.deepEqual(fresh.mergeBy.names, ['id'])
  assert.equal(fresh.maxRowSize, 10)

  // A refused section leaves the options as they were.
  assert.throws(() => {
    options.filter = 'venue = '
  }, /expression/)
  assert.ok(options.filter.equals("venue = 'XNAS' and id > 5"))
  assert.throws(() => options.withSelect('select'), /projection/)
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
  // shared name still names the inferred root.
  const text = new IOBase(path.join(root, 'events.log'))
  text.writeText('first\nsecond\n')
  const textOptions = text.recordOptions().withName('events')
  const textField = text.readArrowField(textOptions)
  assert.equal(textField.name, 'events')
  assert.deepEqual(textField.entries(), [])
})
