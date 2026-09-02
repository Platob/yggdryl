'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')

const arrow = require('apache-arrow')

const { Field, IOBase, fields, iceberg } = require('yggdryl')

// A caller's own storage, the way a Node program already holds one: a Map of
// paths to bytes, reached through `this`. A directory here is a prefix, which
// is what an object store means by one.
function memory() {
  return {
    typeName: 'memory',
    files: new Map(),
    fileInfo(location) {
      const key = trimmed(location)
      const stored = this.files.get(key)
      if (stored !== undefined) {
        return { path: key, kind: 'file', size: BigInt(stored.length) }
      }
      if (key === '' || this.under(key).length !== 0) {
        return { path: key, kind: 'directory' }
      }
      // Arrow's own spelling of a path holding nothing yet.
      return { path: key, kind: 'not-found' }
    },
    list(location, recursive) {
      const prefix = trimmed(location)
      const nested = prefix === '' ? '' : `${prefix}/`
      const directories = new Set()
      const found = []
      for (const name of this.under(prefix)) {
        const parts = name.slice(nested.length).split('/')
        // Every prefix between here and the file is a directory, which is
        // what an object store means by one.
        for (let depth = 1; depth < parts.length; depth += 1) {
          if (recursive || depth === 1) {
            directories.add(nested + parts.slice(0, depth).join('/'))
          }
        }
        if (parts.length !== 1 && !recursive) continue
        found.push({ path: name, kind: 'file', size: BigInt(this.files.get(name).length) })
      }
      for (const name of directories) found.push({ path: name, kind: 'directory' })
      return found
    },
    readRange(location, offset, length) {
      const stored = this.files.get(trimmed(location))
      if (stored === undefined) return null
      const start = Number(offset)
      return stored.subarray(start, start + length)
    },
    writeFull(location, bytes) {
      this.files.set(trimmed(location), Buffer.from(bytes))
    },
    createDir() {
      // A directory is a prefix, so there is nothing to create.
    },
    deleteFile(location) {
      this.files.delete(trimmed(location))
    },
    under(prefix) {
      const nested = prefix === '' ? '' : `${prefix}/`
      return [...this.files.keys()].filter((name) => name.startsWith(nested))
    },
  }
}

// The same six calls over `node:fs`. It throws where the vtable answers -
// `ENOENT` rather than nothing - which is exactly what the boundary has to
// turn back into the laziness contract.
function local() {
  return {
    typeName: 'local',
    fileInfo(location) {
      const stats = fs.statSync(location, { throwIfNoEntry: false })
      if (stats === undefined) return { path: location, kind: 'not-found' }
      if (stats.isDirectory()) return { path: location, kind: 'directory' }
      // A number is read where it is exact, so a handler need not reach for
      // a bigint to report a small file.
      return { path: location, kind: 'file', size: stats.size }
    },
    list(location, recursive) {
      return fs
        .readdirSync(location, { recursive, withFileTypes: true })
        .map((entry) => {
          const full = path.join(entry.parentPath ?? entry.path, entry.name)
          return entry.isDirectory()
            ? { path: full, kind: 'directory' }
            : { path: full, kind: 'file', size: BigInt(fs.statSync(full).size) }
        })
    },
    readRange(location, offset, length) {
      const buffer = Buffer.alloc(length)
      const descriptor = fs.openSync(location, 'r')
      try {
        return buffer.subarray(0, fs.readSync(descriptor, buffer, 0, length, Number(offset)))
      } finally {
        fs.closeSync(descriptor)
      }
    },
    writeFull(location, bytes) {
      fs.writeFileSync(location, bytes)
    },
    createDir(location) {
      fs.mkdirSync(location, { recursive: true })
    },
    deleteFile(location) {
      fs.rmSync(location, { force: true })
    },
  }
}

function trimmed(location) {
  return location.replace(/^\/+|\/+$/g, '')
}

function scratch() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-arrowfs-'))
}

function names(handles) {
  return handles.map((handle) => handle.name).sort()
}

function trades() {
  return new arrow.Table({
    id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
    symbol: arrow.vectorFromArray(['AAPL', 'MSFT'], new arrow.Utf8()),
  })
}

test('a handler-backed handle is an ordinary handle', () => {
  const handle = IOBase.fromArrowFs(memory(), 'bucket/trades.parquet')

  // The file system's own name is the scheme its locations carry.
  assert.equal(handle.toString(), 'memory://bucket/trades.parquet')
  assert.equal(handle.name, 'trades.parquet')
  assert.equal(handle.mediaType.toString(), 'application/vnd.apache.parquet')

  // Per the laziness contract nothing was created, so nothing is there.
  assert.ok(!handle.exists())
  assert.equal(handle.size, 0)
  assert.equal(handle.readBytes().length, 0)
})

test('the constructor infers a file system first argument', () => {
  const handler = memory()
  const inferred = new IOBase(handler, 'bucket/inferred.bin')
  const explicit = IOBase.fromArrowFs(handler, 'bucket/inferred.bin')
  assert.equal(inferred.toString(), explicit.toString())

  inferred.writeText('same')
  inferred.flush()
  assert.equal(explicit.readText(), 'same')

  // A file system with nowhere to resolve, and a location with a file system
  // that is not one, are both refused by name.
  assert.throws(() => new IOBase(handler), /a path on the file system as the second argument/)
  assert.throws(() => new IOBase('some/path', 'another'), /Arrow file system handler to resolve/)
})

test('a handler missing a method is refused by that method', () => {
  const partial = { ...memory() }
  delete partial.readRange

  assert.throws(
    () => IOBase.fromArrowFs(partial, 'bucket/key.bin'),
    /readRange\(path, offset, length\)/,
  )
  assert.throws(() => IOBase.fromArrowFs({}, 'bucket/key.bin'), /fileInfo\(path\)/)
  // A method that is not callable is no method at all.
  assert.throws(
    () => IOBase.fromArrowFs({ ...memory(), list: 'everything' }, 'bucket/key.bin'),
    /list\(path, recursive\)/,
  )
})

test('bytes round trip, and a write publishes when the handle is flushed', () => {
  const handler = memory()
  const handle = IOBase.fromArrowFs(handler, 'bucket/staged.bin')

  // Positional writes are pieces of a value, so they stage: an Arrow file
  // system replaces whole files rather than writing ranges, and must never be
  // handed a half-written one.
  handle.pwrite(0, Buffer.from('pend'))
  handle.pwrite(4, Buffer.from('ing'))
  assert.ok(!handler.files.has('bucket/staged.bin'))
  // The handle itself reads what it staged, and says a file is there now.
  assert.equal(handle.readText(), 'pending')
  assert.ok(handle.isFile())

  handle.flush()
  assert.equal(handler.files.get('bucket/staged.bin').toString(), 'pending')

  // A whole-value write needs no scope at all: it is one replacement, so it
  // publishes when it finishes.
  const whole = IOBase.fromArrowFs(handler, 'bucket/whole.bin')
  whole.writeText('published')
  assert.equal(handler.files.get('bucket/whole.bin').toString(), 'published')

  // Positional access needs no mode here either, and the ranged read maps
  // straight onto one `readRange`.
  assert.equal(handle.pwrite(0, Buffer.from('PENDING')), 7)
  handle.flush()
  assert.equal(handle.readRangeBytes(0, 3).toString(), 'PEN')
  assert.equal(handle.appendBytes(Buffer.from('!')), 7)
  handle.flush()
  assert.equal(handler.files.get('bucket/staged.bin').toString(), 'PENDING!')

  handle.unlink()
  handle.flush()
  assert.equal(handle.readBytes().length, 0)
})

test('a size crosses as a bigint and as an exact number', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  fs.writeFileSync(path.join(root, 'sized.bin'), 'symbol,price')

  // The `node:fs` handler reports a plain number, the memory one a bigint;
  // both are the same length to the core.
  assert.equal(IOBase.fromArrowFs(local(), path.join(root, 'sized.bin')).size, 12)

  const handler = memory()
  handler.files.set('bucket/sized.bin', Buffer.from('symbol,price'))
  assert.equal(IOBase.fromArrowFs(handler, 'bucket/sized.bin').size, 12)

  // A length no length can be is refused rather than truncated. The write is
  // what asks: it loads the stored value before staging over it.
  const lying = {
    ...memory(),
    fileInfo: (location) => ({ path: location, kind: 'file', size: -1n }),
  }
  assert.throws(
    () => IOBase.fromArrowFs(lying, 'bucket/key.bin').writeText('AAPL'),
    /unsigned 64-bit integer/,
  )
})

test('folders list, glob, and carry the file system', () => {
  const handler = memory()
  for (const year of ['2024', '2025']) {
    for (const month of ['01', '02']) {
      handler.files.set(`lake/year=${year}/month=${month}/part-0.parquet`, Buffer.from('PAR1'))
      handler.files.set(`lake/year=${year}/month=${month}/notes.txt`, Buffer.from('notes'))
    }
  }
  const lake = IOBase.fromArrowFs(handler, 'lake')

  assert.ok(lake.isDir())
  assert.deepEqual(names([...lake.iterdir()]), ['year=2024', 'year=2025'])
  assert.deepEqual(names([...lake]), ['year=2024', 'year=2025'])
  assert.equal([...lake.ls(true)].length, 14)
  assert.equal([...lake.glob('**/*.parquet')].length, 4)
  assert.equal([...lake.rglob('*.parquet')].length, 4)
  assert.equal([...lake.glob('year=2024/**/*.parquet')].length, 2)

  // Every handle a folder hands back still stands on the same file system.
  const leaf = lake.joinpath('year=2024', 'month=01', 'part-0.parquet')
  assert.equal(leaf.readText(), 'PAR1')
  assert.equal(leaf.parent.name, 'month=01')
  assert.equal(leaf.toString(), 'memory://lake/year=2024/month=01/part-0.parquet')
  assert.deepEqual(leaf.partitions, [
    { column: 'year', value: '2024' },
    { column: 'month', value: '01' },
  ])
  assert.equal([...lake.childrenWhere({ year: '2024' })].length, 4)

  // A rebuilt handle keeps the file system, which its location alone would
  // not say: a `memory:` URL names no backend.
  assert.equal(IOBase.from(leaf).readText(), 'PAR1')
  assert.equal(new IOBase(leaf).readText(), 'PAR1')
})

test('a missing location reads empty rather than throwing', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  // `node:fs` throws `ENOENT` where the vtable answers with nothing, so this
  // is the boundary turning a handler's failure back into the contract.
  const absent = IOBase.fromArrowFs(local(), path.join(root, 'nowhere', 'absent.arrows'))
  assert.ok(!absent.exists())
  assert.equal(absent.size, 0)
  assert.equal(absent.readBytes().length, 0)
  assert.deepEqual([...absent.ls(true)], [])
  // Removing what was never there is what was asked for.
  absent.unlink()

  // A write creates the file and the parents it needs.
  const nested = IOBase.fromArrowFs(local(), path.join(root, 'deep', 'nested', 'trades.txt'))
  nested.writeText('AAPL')
  nested.flush()
  assert.equal(fs.readFileSync(path.join(root, 'deep', 'nested', 'trades.txt'), 'utf8'), 'AAPL')
})

test('records round trip through the wrapper, in both encodings', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))

  for (const name of ['trades.arrows', 'trades.parquet']) {
    const handle = IOBase.fromArrowFs(local(), path.join(root, 'lake', name))
    handle.overwriteArrowTable(trades())
    handle.flush()

    // What the wrapper published is a file any other reader can open, so the
    // local backend reads the same rows off the same bytes.
    const outside = new IOBase(path.join(root, 'lake', name))
    assert.deepEqual(outside.readBytes(), handle.readBytes())
    assert.equal(outside.readArrowReader().intoTable().numRows, 2)
    outside.close()

    const table = IOBase.fromArrowFs(local(), path.join(root, 'lake', name))
      .readArrowReader()
      .intoTable()
    assert.equal(table.numRows, 2)
    assert.deepEqual(
      table.schema.fields.map((field) => field.name),
      ['id', 'symbol'],
    )
    assert.deepEqual([...table.getChild('symbol')], ['AAPL', 'MSFT'])

    // Appending keeps its explicit intent and reads-adds-rewrites.
    handle.appendArrowTable(trades())
    handle.flush()
    assert.equal(
      IOBase.fromArrowFs(local(), path.join(root, 'lake', name))
        .readArrowReader()
        .intoTable().numRows,
      4,
    )
  }
})

test('records round trip over storage that is only a Map', () => {
  const handler = memory()
  const handle = IOBase.fromArrowFs(handler, 'bucket/trades.arrows')

  handle.overwriteArrowTable(trades())
  handle.flush()

  // Every byte of that table went through the caller's own handler.
  assert.ok(handler.files.has('bucket/trades.arrows'))
  assert.equal(handle.readArrowField().dtype.length, 2)
  assert.equal(
    IOBase.fromArrowFs(handler, 'bucket/trades.arrows').readArrowReader().intoTable().numRows,
    2,
  )

  // Text lines read off a handler-backed handle too: the iterator rebuilds
  // the handle, which is where keeping the file system matters.
  const lines = IOBase.fromArrowFs(handler, 'bucket/rows.jsonl')
  lines.writeText('{"id":1}\n{"id":2}\n')
  lines.flush()
  assert.deepEqual([...lines.readLines()], ['{"id":1}', '{"id":2}'])
})

test("a handler that throws surfaces its own message", () => {
  const refusing = {
    ...memory(),
    fileInfo: (location) => ({ path: location, kind: 'file', size: 8n }),
    readRange() {
      throw new Error('the bucket refused the request: 403 Forbidden')
    },
  }
  const handle = IOBase.fromArrowFs(refusing, 'bucket/key.bin')
  // The file is there as far as the handler is concerned, so the refusal is
  // a real failure rather than absence, and its own words cross unchanged.
  assert.throws(() => handle.readBytes(), /403 Forbidden/)

  const failing = {
    ...memory(),
    fileInfo() {
      throw new Error('the credential chain is empty')
    },
  }
  assert.throws(
    () => IOBase.fromArrowFs(failing, 'bucket/key.bin').writeText('AAPL'),
    /the credential chain is empty/,
  )

  // A kind the core has no name for is refused naming the vocabulary.
  const inventing = {
    ...memory(),
    fileInfo: (location) => ({ path: location, kind: 'blob' }),
  }
  assert.throws(
    () => IOBase.fromArrowFs(inventing, 'bucket/key.bin').writeText('AAPL'),
    /memory, file, directory, table, namespace, catalog, unknown/,
  )
})

test('an Iceberg table lives on a file system a caller wrote', () => {
  const handler = memory()
  const warehouse = IOBase.fromArrowFs(handler, 'warehouse/trades')
  const schema = iceberg.assignFieldIds(
    fields.struct('row', [Field.from('id: int64'), Field.from('symbol: utf8')], {
      nullable: false,
    }),
  )

  const table = iceberg.Table.create(warehouse, schema)
  table.append(trades())

  const rows = table.scan().intoTable()
  assert.equal(rows.numRows, 2)
  assert.deepEqual(
    rows.schema.fields.map((field) => field.name),
    ['id', 'symbol'],
  )

  // A table is a folder, and this one is a folder on the caller's own
  // storage: every byte of it went through those same six calls.
  const stored = [...handler.files.keys()]
  assert.ok(stored.some((name) => name.endsWith('.metadata.json')))
  assert.ok(stored.some((name) => name.endsWith('.parquet')))
})

test('a scope publishes the staged value it wrote', () => {
  const handler = memory()

  // `using` binds to this, and it is what hands a staged whole value to a
  // file system that replaces files rather than writing ranges. The symbol
  // is called directly because the `using` *declaration* is newer than the
  // Node this suite runs on, while the protocol it calls is not.
  {
    const handle = IOBase.fromArrowFs(handler, 'bucket/scoped.bin')
    handle.pwrite(0, Buffer.from('AAPL'))
    // Still staged: the handler has not been asked to store anything yet.
    assert.equal(handler.files.has('bucket/scoped.bin'), false)
    assert.equal(typeof handle[Symbol.dispose], 'function')
    handle[Symbol.dispose]()
  }

  assert.deepEqual(
    Buffer.from(handler.files.get('bucket/scoped.bin')).toString(),
    'AAPL',
  )
  assert.equal(IOBase.fromArrowFs(handler, 'bucket/scoped.bin').readText(), 'AAPL')
})

test('open and close bracket the staged state without creating anything', () => {
  const handler = memory()
  const handle = IOBase.fromArrowFs(handler, 'bucket/absent.bin')

  // Opening a resource that does not exist yet caches nothing to publish.
  handle.open()
  handle.close()
  assert.equal(handler.files.has('bucket/absent.bin'), false)

  handle.writeText('later')
  assert.equal(handle.opened(), true)
  assert.equal(handle.closed(), false)
  handle.close()
  assert.equal(handle.opened(), false)
  assert.equal(handle.closed(), true)
  assert.equal(handle.readText(), 'later')
})

test('mkdir creates the container on the same file system', () => {
  const handler = memory()
  const handle = IOBase.fromArrowFs(handler, 'bucket/lake')

  // A location does not say which backend it belongs to, so mkdir must not
  // quietly rebuild the handle on the local disk.
  handle.mkdir()
  const child = handle.joinpath(['part-0.bin'])
  child.writeText('AAPL')
  child.close()

  assert.equal(Buffer.from(handler.files.get('bucket/lake/part-0.bin')).toString(), 'AAPL')
  assert.equal(fs.existsSync('bucket/lake'), false)
})

test('a table hands back a root on its own file system', () => {
  const handler = memory()
  const warehouse = IOBase.fromArrowFs(handler, 'warehouse/trades')
  const schema = iceberg.assignFieldIds(
    fields.struct('row', [Field.from('id: int64'), Field.from('symbol: utf8')], {
      nullable: false,
    }),
  )

  const table = iceberg.Table.create(warehouse, schema)
  table.append(trades())

  // The root is the folder the table actually lives in, not the local path
  // its recorded location happens to spell.
  const root = table.root
  assert.equal(root.isDir(), true)
  assert.ok([...root.ls()].some((entry) => entry.name === 'metadata'))
  assert.equal([...root.glob('data/**/*.parquet')].length, 1)
})
