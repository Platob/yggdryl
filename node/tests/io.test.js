'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')

const { IOBase, Url } = require('yggdryl')

// A small Hive-partitioned lake with one private staging area, so listing,
// globbing, and partition selection all have something real to answer about.
function lake() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-io-'))
  for (const year of ['2024', '2025']) {
    for (const month of ['01', '02']) {
      const leaf = path.join(root, `year=${year}`, `month=${month}`)
      fs.mkdirSync(leaf, { recursive: true })
      fs.writeFileSync(path.join(leaf, 'part-0.parquet'), 'parquet')
      fs.writeFileSync(path.join(leaf, 'notes.txt'), 'notes')
    }
  }
  const staging = path.join(root, '.staging')
  fs.mkdirSync(staging)
  fs.writeFileSync(path.join(staging, 'part-0.parquet'), 'draft')
  return root
}

function scratch() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-io-'))
}

function names(handles) {
  return handles.map((handle) => handle.name).sort()
}

test('a handle reports what is there and is inferred from every spelling', (t) => {
  const root = lake()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const handle = new IOBase(root)

  assert.ok(handle.exists())
  assert.ok(handle.isDir())
  assert.ok(!handle.isFile())
  assert.equal(handle.name, path.basename(root))
  assert.equal(handle.mediaType.base.toString(), 'inode/directory')

  // A string, a native Url, and another handle all name the same location.
  assert.equal(IOBase.from(Url.fromPath(root)).toString(), handle.toString())
  assert.equal(new IOBase(handle).toString(), handle.toString())
  assert.equal(handle.url.toString(), Url.fromPath(root).toString())
  assert.equal(handle.toPath(), Url.fromPath(root).toPath())
})

test('a missing location is empty rather than an error', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const absent = new IOBase(path.join(root, 'absent.arrows'))

  assert.ok(!absent.exists())
  // Reads skip, so probing a location needs no existence check first.
  assert.equal(absent.readBytes().length, 0)
  assert.equal(absent.size, 0)
})

test('children are resolved the way path segments are', (t) => {
  const root = lake()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const stepwise = new IOBase(root).joinpath('year=2024').joinpath('month=01')
  const atOnce = new IOBase(root).joinpath('year=2024', 'month=01')
  const asArray = new IOBase(root).joinpath(['year=2024', 'month=01'])

  assert.equal(stepwise.toString(), atOnce.toString())
  assert.equal(asArray.toString(), atOnce.toString())
  assert.equal(atOnce.joinpath('part-0.parquet').readText(), 'parquet')
  assert.equal(atOnce.joinpath('part-0.parquet').parent.name, 'month=01')
  // Joining nothing is the same location.
  assert.equal(atOnce.joinpath().toString(), atOnce.toString())
  assert.throws(() => atOnce.joinpath(7), TypeError)
})

test('iterdir skips private entries by default', (t) => {
  const root = lake()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const handle = new IOBase(root)

  assert.deepEqual(names(handle.iterdir()), ['year=2024', 'year=2025'])
  assert.ok(names(handle.iterdir(true)).includes('.staging'))
  // Iterating the handle itself is iterdir, as iterating a directory is.
  assert.deepEqual(names([...handle]), ['year=2024', 'year=2025'])
})

test('ls descends only when it is asked to', (t) => {
  const root = lake()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const handle = new IOBase(root)

  assert.equal(handle.ls().length, 2)
  // Two years, four months, and eight leaves.
  assert.equal(handle.ls(true).length, 14)
  assert.equal(handle.ls(true, true).length, 16)
  // A leaf contains nothing rather than failing to be listed.
  assert.deepEqual(handle.joinpath('year=2024', 'month=01', 'notes.txt').ls(true), [])
})

test('glob and rglob select the same leaves', (t) => {
  const root = lake()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const handle = new IOBase(root)

  assert.equal(handle.glob('**/*.parquet').length, 4)
  assert.equal(handle.rglob('*.parquet').length, 4)
  assert.equal(handle.glob('year=2024/**/*.parquet').length, 2)
  // One plain segment stays at one level, where there are no leaves.
  assert.deepEqual(handle.glob('*.parquet'), [])
  assert.equal(handle.rglob('*.parquet', true).length, 5)
})

test('a write creates and a read returns it', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const handle = new IOBase(path.join(root, 'trades.txt'))

  assert.equal(handle.writeText('AAPL'), 4)
  assert.equal(handle.readText(), 'AAPL')
  assert.ok(handle.exists())
  assert.equal(handle.size, 4)

  assert.equal(handle.writeBytes(Buffer.from('MSFT,1')), 6)
  assert.deepEqual(handle.readBytes(), Buffer.from('MSFT,1'))

  handle.unlink()
  assert.equal(handle.readBytes().length, 0)
})

test('positional access needs no mode and rejects impossible offsets', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const handle = new IOBase(path.join(root, 'positional.bin'))
  handle.writeBytes(Buffer.from('symbol,price'))

  // Random access is the contract, so there is nothing to open or seek.
  assert.equal(handle.pread(0, 6).toString(), 'symbol')
  assert.equal(handle.pwrite(0, Buffer.from('SYMBOL')), 6)
  assert.equal(handle.readText(), 'SYMBOL,price')
  assert.equal(handle.append(Buffer.from('!')), 12)
  assert.equal(handle.size, 13)

  handle.truncate(6)
  assert.equal(handle.readText(), 'SYMBOL')
  assert.throws(() => handle.pread(-1, 4), /offset must be a non-negative whole number/)
  assert.throws(() => handle.pread(1.5, 4), /offset/)
  assert.throws(() => handle.truncate(Number.NaN), /size/)
})

test('mkdir and touch bring a location into being', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const folder = new IOBase(path.join(root, 'nested', 'deep'))
  folder.mkdir()
  assert.ok(folder.isDir())

  const leaf = folder.joinpath('empty.arrows')
  leaf.touch()
  assert.ok(leaf.exists())
  assert.equal(leaf.size, 0)

  leaf.writeText('kept')
  leaf.touch()
  // `touch` never truncates an existing leaf.
  assert.equal(leaf.readText(), 'kept')
  assert.throws(() => folder.touch(), /got the directory/)
})

test('bytes move between two handles without a temporary copy', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const source = new IOBase(path.join(root, 'source.csv'))
  source.writeText('symbol,price\n')

  const target = IOBase.fromBytes()
  assert.equal(source.copyInto(target), 13)
  assert.equal(target.readText(), 'symbol,price\n')
  source.flush()
})

test('a memory handle needs no location', () => {
  const handle = IOBase.fromBytes(Buffer.from('AAPL'))

  assert.equal(handle.readText(), 'AAPL')
  assert.equal(handle.size, 4)
  // A buffer still has an identity, but not one the file system knows.
  assert.equal(handle.url.scheme, 'mem')
  assert.throws(() => handle.toPath(), /only a file URI/)
  assert.throws(() => handle.mkdir(), /only a file URI/)
  assert.equal(IOBase.fromBytes().size, 0)
})

test('a leaf knows the partitions above it', (t) => {
  const root = lake()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const leaf = new IOBase(root).joinpath('year=2024', 'month=01', 'part-0.parquet')

  assert.deepEqual(leaf.partitions, [
    { column: 'year', value: '2024' },
    { column: 'month', value: '01' },
  ])
  assert.deepEqual(IOBase.fromBytes().partitions, [])
})

test('childrenWhere selects the parts to rewrite', (t) => {
  const root = lake()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const handle = new IOBase(root)

  const year = handle.childrenWhere({ year: '2024' })
  assert.equal(year.length, 4)
  assert.ok(year.every((entry) => entry.isFile()))

  // A Map, an entry array, and a plain object spell the same set of pairs.
  assert.equal(handle.childrenWhere(new Map([['year', '2024']])).length, 4)
  assert.equal(handle.childrenWhere([['year', '2024']]).length, 4)
  assert.equal(
    handle.childrenWhere([{ column: 'year', value: '2024' }]).length,
    4,
  )

  const both = handle.childrenWhere([
    ['year', '2024'],
    ['month', '02'],
  ])
  assert.equal(both.length, 2)
  assert.deepEqual(handle.childrenWhere({ year: '1999' }), [])
  // No filter is every leaf.
  assert.equal(handle.childrenWhere({}).length, 8)
  assert.equal(handle.childrenWhere({}, true).length, 9)

  assert.throws(() => handle.childrenWhere('year=2024'), TypeError)
  assert.throws(() => handle.childrenWhere([['year']]), TypeError)
  assert.throws(() => handle.childrenWhere({ year: 2024 }), TypeError)
})

test('readLines streams decoded lines off a handle', (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-lines-'))
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const plain = path.join(root, 'rows.jsonl')
  fs.writeFileSync(plain, '{"id":1}\n{"id":2}\r\n{"id":3}')
  assert.deepEqual([...new IOBase(plain).readLines()], ['{"id":1}', '{"id":2}', '{"id":3}'])

  // A gzip-named resource decodes as a stream; the lines read the same.
  const zlib = require('node:zlib')
  const coded = path.join(root, 'words.txt.gz')
  fs.writeFileSync(coded, zlib.gzipSync('alpha\nbeta\n'))
  assert.deepEqual([...new IOBase(coded).readLines()], ['alpha', 'beta'])

  // Absence is emptiness, exactly as reading zero bytes is.
  assert.deepEqual([...new IOBase(path.join(root, 'missing.txt')).readLines()], [])
})

test('readLines with a pattern groups log entries', (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-loglines-'))
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const target = path.join(root, 'app.log')
  fs.writeFileSync(
    target,
    '2024-02-01 10:00:00.000_000 [ee] [alpha] boom\n  at frame one\n' +
      '2024-02-01 10:00:01.000_000 [ii] [beta] fine\n',
  )
  const entries = [...new IOBase(target).readLines('^\\d{4}-\\d{2}-\\d{2} \\d{2}:\\d{2}:\\d{2}')]
  assert.deepEqual(entries, [
    '2024-02-01 10:00:00.000_000 [ee] [alpha] boom\n  at frame one',
    '2024-02-01 10:00:01.000_000 [ii] [beta] fine',
  ])
})

test('readArrowLines projects matched records into typed batches', (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-arrowlines-'))
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const target = path.join(root, 'app.log')
  fs.writeFileSync(
    target,
    'preamble carried from rotation\n' +
      '2024-02-01 10:00:00.000_000 [ee] [alpha] boom\n  at frame one\n' +
      '2024-02-01 10:00:01.500 [ii] [beta] fine\n',
  )
  const pattern =
    '^\\d{4}-\\d{2}-\\d{2} \\d{2}:\\d{2}:\\d{2}\\S* \\[(?<level>[^\\]]+)\\] \\[(?<logger>[^\\]]+)\\]'
  const table = new IOBase(target)
    .readArrowLines(pattern, { customFields: { venue: 'XNAS' } })
    .toTable()
  assert.equal(table.numRows, 3)
  assert.deepEqual(
    table.schema.fields.map((field) => field.name),
    [
      'url',
      'rownum',
      'date',
      'time',
      'unix',
      'hash',
      'header',
      'message',
      'offset',
      'lines',
      'level',
      'logger',
      'venue',
    ],
  )
  const messages = [...table.getChild('message')]
  assert.deepEqual(messages, [
    'preamble carried from rotation',
    'boom\n  at frame one',
    'fine',
  ])
  // The preamble has no header: its level is null, its unix is null; the
  // matched rows carry naive nanoseconds and the constant venue stamp.
  assert.deepEqual([...table.getChild('level')], [null, 'ee', 'ii'])
  const unix = [...table.getChild('unix')]
  assert.equal(unix[0], null)
  assert.equal(unix[1], 1_706_781_600_000_000_000n)
  assert.deepEqual([...table.getChild('venue')], ['XNAS', 'XNAS', 'XNAS'])

  // A gzip log reads the same rows through its streaming decode.
  const zlib = require('node:zlib')
  const coded = path.join(root, 'app2.log.gz')
  fs.writeFileSync(coded, zlib.gzipSync('2024-02-01 10:00:00 [ii] [a] fine\n'))
  assert.equal(new IOBase(coded).readArrowLines(pattern).toTable().numRows, 1)

  // Absence reads as zero rows with the schema still answered.
  const empty = new IOBase(path.join(root, 'missing.log')).readArrowLines(pattern)
  assert.equal(empty.toTable().numRows, 0)

  // An in-memory handle parses exactly as a file does.
  const memory = IOBase.fromBytes(Buffer.from('2024-02-01 12:00:00 [ww] [m] held\n'))
  assert.equal(memory.readArrowLines(pattern).toTable().numRows, 1)

  // Inputs that would silently misparse are refused instead.
  assert.throws(() => new IOBase(target).readArrowLines(pattern, { customFields: 'venue' }), TypeError)
  assert.throws(() => new IOBase(target).readArrowLines(pattern, { batchSize: -1 }), TypeError)
  assert.throws(() => new IOBase(target).readArrowLines(pattern, { batchSize: 1.5 }), TypeError)
})

test('named captures type themselves and declarations override', (t) => {
  const { schemaFromPattern } = require('yggdryl')
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-typedlines-'))
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const target = path.join(root, 'typed.log')
  fs.writeFileSync(target, '2024-02-01 10:00:00 [42] (info) qty=1.50\n')
  const pattern =
    '^\\d{4}-\\d{2}-\\d{2} \\d{2}:\\d{2}:\\d{2} \\[(?<threadId>\\d+)\\] \\((?<logLevel>\\w+)\\) qty=(?<qty>[0-9.]+)'

  // The standalone builder answers the emitted root without a reader:
  // `threadId` typed off its own `\d+` sub-pattern, `qty` by declaration.
  const schema = schemaFromPattern(pattern, { captureTypes: { qty: 'decimal(9, 2)' } })
  assert.equal(schema.name, 'row')
  assert.equal(String(schema.dataType.get('threadId').dataType), 'int64')
  assert.equal(String(schema.dataType.get('qty').dataType), 'decimal128(9,2)')

  const table = new IOBase(target)
    .readArrowLines(pattern, { captureTypes: { qty: 'decimal(9, 2)' } })
    .toTable()
  assert.deepEqual([...table.getChild('threadId')], [42n])
  assert.deepEqual([...table.getChild('logLevel')], ['info'])
})

test('writeLines and appendLines stream and round-trip', (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-writelines-'))
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const target = new IOBase(path.join(root, 'out.log'))
  // A generator, never an array the binding materializes first.
  target.writeLines(
    (function* rows() {
      for (let index = 0; index < 1_000; index += 1) {
        yield `row-${index}`
      }
    })(),
  )
  target.appendLines(['tail'])

  assert.ok(target.readBytes().toString().endsWith('row-999\ntail\n'))
  const records = [...target.readLines()]
  assert.equal(records.length, 1_001)
  assert.equal(records.at(-1), 'tail')

  // A pinned terminator is written verbatim and read back exactly.
  const pinned = new IOBase(path.join(root, 'crlf.log'))
  pinned.writeLines(['one', 'two'], { linesep: '\\r\\n' })
  assert.equal(pinned.readBytes().toString(), 'one\r\ntwo\r\n')
  assert.deepEqual([...pinned.readLines({ linesep: '\\r\\n' })], ['one', 'two'])

  // Bytes pass as themselves, and a bare string is refused rather than
  // silently written one character per record.
  const bytes = new IOBase(path.join(root, 'bytes.log'))
  bytes.writeLines([Buffer.from('alpha'), 'beta'])
  assert.deepEqual([...bytes.readLines()], ['alpha', 'beta'])
  assert.throws(() => bytes.writeLines('alpha'), TypeError)
})

test('a reader is fully described by a configuration document', (t) => {
  const { schemaFromPattern, yaml } = require('yggdryl')
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-lineconfig-'))
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const target = path.join(root, 'app.log')
  fs.writeFileSync(
    target,
    '2024-02-01T10:00:00 [ERROR] boom\n\tat Handler.invoke(Handler.java:42)\n' +
      '2024-02-01T10:00:01 [INFO] fine\n',
  )

  // No JavaScript in the loop: the document *is* the reader.
  const options = yaml.loads(
    [
      "pattern: '^(?<stamp>\\S+) \\[(?<level>[A-Z]+)\\]'",
      'byte_size: 1048576',
      'batch_size: 4096',
      'timestamp_capture: stamp',
      'custom_fields:',
      '  source: gateway',
    ].join('\n'),
  )

  // The schema answers from the document alone, with no resource in sight.
  const schema = schemaFromPattern(options)
  assert.equal(schema.name, 'row')
  assert.equal(String(schema.dataType.get('source').dataType), 'utf8')

  const table = new IOBase(target).readArrowLines(options).toTable()
  assert.equal(table.numRows, 2)
  assert.deepEqual([...table.getChild('level')], ['ERROR', 'INFO'])
  assert.deepEqual([...table.getChild('source')], ['gateway', 'gateway'])
})

test('log mode needs no expression, and both batch bounds apply', (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-logmode-'))
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const target = path.join(root, 'app.log')
  const rows = []
  for (let index = 0; index < 50; index += 1) {
    rows.push(`2024-02-01T10:00:00 [INFO] row ${index}`)
  }
  fs.writeFileSync(target, `${rows.join('\n')}\n`)
  const handle = new IOBase(target)

  // The fixed token columns come from the closed table, not from captures.
  const table = handle.readArrowLines({ logs: true }).toTable()
  assert.equal(table.numRows, 50)
  assert.deepEqual([...table.getChild('level')].slice(0, 2), ['INFO', 'INFO'])

  const byRows = [...handle.readArrowLines({ logs: true, batchSize: 10 })]
  assert.deepEqual(
    byRows.map((batch) => batch.numRows),
    [10, 10, 10, 10, 10],
  )

  // `byteSize` counts decoded input bytes, so it closes batches sooner here;
  // whichever bound trips first wins, and every record still arrives.
  const byBytes = [...handle.readArrowLines({ logs: true, byteSize: 100 })]
  const counts = byBytes.map((batch) => batch.numRows)
  assert.equal(
    counts.reduce((total, count) => total + count, 0),
    50,
  )
  assert.ok(Math.max(...counts) < 10)

  // An option the core does not know is refused by name, not ignored.
  assert.throws(() => handle.readArrowLines({ 'batch-size': 1 }), TypeError)
  assert.throws(() => handle.readLines({ timezone: 'Not/AZone' }), Error)
})

test('a cursor shares the handle and owns its position', () => {
  const handle = IOBase.fromBytes()
  const cursor = handle.cursor()

  assert.equal(cursor.write(Buffer.from('symbol,')), 7)
  assert.equal(cursor.write(Buffer.from('price\n')), 6)
  assert.equal(cursor.tell(), 13)
  // The write landed on the handle itself, not on a copy.
  assert.equal(handle.readBytes().toString(), 'symbol,price\n')

  assert.equal(cursor.seek(7), 7)
  assert.equal(cursor.read(5).toString(), 'price')
  assert.equal(cursor.read().toString(), '\n')
  assert.equal(cursor.read().length, 0)

  // Two cursors advance independently.
  const ahead = handle.cursor(7)
  assert.equal(ahead.read(5).toString(), 'price')
  assert.equal(cursor.tell(), 13)
})

test('a handle reports the content coding its own name declares', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  // The question a caller asks here is whether there is anything to undo, so
  // bytes carrying no coding answer null rather than the identity the core
  // spells internally.
  assert.equal(new IOBase(path.join(root, 'trades.json')).codec, null)
  assert.equal(new IOBase(path.join(root, 'trades.json.gz')).codec, 'gzip')
  assert.equal(new IOBase(path.join(root, 'trades.jsonl.zst')).codec, 'zstd')
  // A resource that does not exist still has a name, and a buffer has none.
  assert.equal(new IOBase(path.join(root, 'absent.txt.gz')).codec, 'gzip')
  assert.equal(IOBase.fromBytes(Buffer.from('AAPL')).codec, null)
})

test('compressInto and decompressInto round-trip through a real .gz', (t) => {
  const zlib = require('node:zlib')
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  const plain = new IOBase(path.join(root, 'trades.json'))
  plain.writeText('{"id":1}')

  // The target's own name declares the coding, so gzip is never named twice.
  const coded = new IOBase(path.join(root, 'trades.json.gz'))
  const written = plain.compressInto(coded)
  assert.equal(written, coded.size)
  assert.deepEqual(zlib.gunzipSync(coded.readBytes()), Buffer.from('{"id":1}'))

  const back = new IOBase(path.join(root, 'back.json'))
  assert.equal(coded.decompressInto(back), 8)
  assert.equal(back.readText(), '{"id":1}')

  // A buffer has no name to declare anything, so it is told - and what it was
  // told is recorded on its media type, so reading it back needs no argument.
  const memory = IOBase.fromBytes()
  assert.equal(plain.compressInto(memory, 'gzip', 9), memory.size)
  assert.equal(memory.codec, 'gzip')
  assert.equal(memory.decompressInto(IOBase.fromBytes()), 8)

  // A target whose name declares nothing is refused rather than copied
  // uncompressed, because a coding nobody named is a coding nobody can decode
  // by name later; a coding no codec answers to is refused naming the ones
  // that exist.
  const flat = new IOBase(path.join(root, 'copy.json'))
  assert.throws(
    () => plain.compressInto(flat),
    /expected a target declaring a content coding, got application\/json; pass a codec/,
  )
  assert.equal(fs.existsSync(path.join(root, 'copy.json')), false)
  assert.throws(
    () => plain.compressInto(flat, 'nonsense'),
    /expected one of identity, gzip, zlib, deflate, zstd, got "nonsense"/,
  )

  // A name that lies is caught by the decoder rather than believed.
  const lying = new IOBase(path.join(root, 'lying.json.gz'))
  lying.writeText('not gzip at all')
  assert.throws(() => lying.decompressInto(IOBase.fromBytes()), /invalid gzip header/)
})
