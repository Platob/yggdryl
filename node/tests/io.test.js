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
