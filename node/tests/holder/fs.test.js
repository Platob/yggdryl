'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')

const arrow = require('apache-arrow')

const {
  Field,
  IOBase,
  RecordOptions,
  TextOptions,
  fields,
  iceberg,
} = require('yggdryl')

function failure(code, message) {
  const error = new Error(`${code}: ${message}`)
  error.code = code
  return error
}

function bufferReader(bytes, randomAccess) {
  let position = 0n
  let closed = false
  const open = () => {
    if (closed) throw failure('Unsupported', 'stream is closed')
  }
  const reader = {
    read(length) {
      open()
      const start = Number(position)
      const value = bytes.subarray(start, start + Number(length))
      position += BigInt(value.length)
      return value
    },
    tell() {
      return position
    },
    close() {
      closed = true
    },
    get closed() {
      return closed
    },
  }
  if (randomAccess) {
    reader.readAt = (offset, length) => {
      open()
      return bytes.subarray(Number(offset), Number(offset + length))
    }
    reader.seek = (offset, whence) => {
      open()
      const base =
        whence === 'current'
          ? position
          : whence === 'end'
            ? BigInt(bytes.length)
            : 0n
      const next = base + offset
      if (next < 0n) throw failure('InvalidInput', 'negative seek')
      position = next
      return position
    }
  }
  return reader
}

// A complete synchronous Arrow handler over a Map. Paths are opaque: there is
// deliberately no trimming, URL parsing, or percent decoding here.
function memory(domain = {}) {
  return {
    typeName: 'memory',
    domain,
    files: new Map(),
    directories: new Set(['']),
    metadata: new Map(),
    equals(other) {
      return other?.domain === this.domain
    },
    normalizePath(location) {
      return location
    },
    fileInfo(location) {
      const stored = this.files.get(location)
      if (stored !== undefined) {
        return { path: location, kind: 'file', size: BigInt(stored.length) }
      }
      if (this.directories.has(location) || this.under(location).length !== 0) {
        return { path: location, kind: 'directory' }
      }
      return { path: location, kind: 'not-found' }
    },
    list(selector) {
      const prefix = selector.baseDir
      if (this.fileInfo(prefix).kind === 'not-found') {
        if (selector.allowNotFound) return []
        throw failure('NotFound', prefix)
      }
      const nested = prefix === '' ? '' : `${prefix}/`
      const entries = new Map()
      for (const name of this.under(prefix)) {
        const parts = name.slice(nested.length).split('/')
        for (let depth = 1; depth < parts.length; depth += 1) {
          if (selector.recursive || depth === 1) {
            const directory = nested + parts.slice(0, depth).join('/')
            entries.set(directory, { path: directory, kind: 'directory' })
          }
        }
        if (parts.length === 1 || selector.recursive) {
          entries.set(name, {
            path: name,
            kind: 'file',
            size: BigInt(this.files.get(name).length),
          })
        }
      }
      // The adapter, not the handler, owns deterministic ordering.
      return [...entries.values()].reverse()
    },
    createDir(location, recursive) {
      if (!recursive && location.includes('/')) {
        const parent = location.slice(0, location.lastIndexOf('/'))
        if (this.fileInfo(parent).kind === 'not-found')
          throw failure('NotFound', parent)
      }
      this.directories.add(location)
    },
    deleteDir(location) {
      if (this.under(location).length !== 0)
        throw failure('DirectoryNotEmpty', location)
      if (!this.directories.delete(location))
        throw failure('NotFound', location)
    },
    deleteDirContents(location, missingDirOk) {
      const info = this.fileInfo(location)
      if (info.kind === 'not-found') {
        if (missingDirOk) return
        throw failure('NotFound', location)
      }
      if (info.kind !== 'directory') throw failure('NotADirectory', location)
      for (const name of this.under(location)) this.files.delete(name)
      for (const name of [...this.directories]) {
        if (name.startsWith(`${location}/`)) this.directories.delete(name)
      }
    },
    deleteRootDirContents() {
      this.files.clear()
      this.directories = new Set([''])
    },
    deleteFile(location) {
      if (this.fileInfo(location).kind === 'directory')
        throw failure('IsADirectory', location)
      if (!this.files.delete(location)) throw failure('NotFound', location)
    },
    copyFile(source, target) {
      const bytes = this.files.get(source)
      if (bytes === undefined) throw failure('NotFound', source)
      this.files.set(target, Buffer.from(bytes))
    },
    move(source, target) {
      const bytes = this.files.get(source)
      if (bytes === undefined) throw failure('NotFound', source)
      this.files.set(target, Buffer.from(bytes))
      this.files.delete(source)
    },
    openInputFile(location) {
      const bytes = this.files.get(location)
      if (bytes === undefined) throw failure('NotFound', location)
      return bufferReader(bytes, true)
    },
    openInputStream(location) {
      const bytes = this.files.get(location)
      if (bytes === undefined) throw failure('NotFound', location)
      return bufferReader(bytes, false)
    },
    openOutputStream(location, metadata) {
      return this.writer(location, false, metadata)
    },
    openAppendStream(location, metadata) {
      return this.writer(location, true, metadata)
    },
    writer(location, append, metadata) {
      const chunks =
        append && this.files.has(location) ? [this.files.get(location)] : []
      let position = chunks.reduce(
        (size, chunk) => size + BigInt(chunk.length),
        0n,
      )
      let closed = false
      const publish = () => {
        this.files.set(location, Buffer.concat(chunks))
        if (metadata !== undefined) this.metadata.set(location, { ...metadata })
      }
      return {
        write(bytes) {
          if (closed) throw failure('Unsupported', 'stream is closed')
          chunks.push(Buffer.from(bytes))
          position += BigInt(bytes.length)
          return BigInt(bytes.length)
        },
        tell() {
          return position
        },
        flush() {
          if (closed) throw failure('Unsupported', 'stream is closed')
          publish()
        },
        close() {
          if (closed) return
          publish()
          closed = true
        },
        get closed() {
          return closed
        },
      }
    },
    under(prefix) {
      const nested = prefix === '' ? '' : `${prefix}/`
      return [...this.files.keys()].filter((name) => name.startsWith(nested))
    },
  }
}

const localDomain = {}

function localReader(location) {
  const descriptor = fs.openSync(location, 'r')
  let position = 0n
  let closed = false
  const readAt = (offset, length) => {
    if (closed) throw failure('Unsupported', 'stream is closed')
    const bytes = Buffer.alloc(Number(length))
    const read = fs.readSync(descriptor, bytes, 0, bytes.length, Number(offset))
    return bytes.subarray(0, read)
  }
  return {
    read(length) {
      const bytes = readAt(position, length)
      position += BigInt(bytes.length)
      return bytes
    },
    readAt,
    seek(offset, whence) {
      const base =
        whence === 'current'
          ? position
          : whence === 'end'
            ? fs.fstatSync(descriptor, { bigint: true }).size
            : 0n
      position = base + offset
      return position
    },
    tell() {
      return position
    },
    close() {
      if (!closed) fs.closeSync(descriptor)
      closed = true
    },
    get closed() {
      return closed
    },
  }
}

function localWriter(location, append) {
  const descriptor = fs.openSync(location, append ? 'a' : 'w')
  let position = append ? fs.fstatSync(descriptor, { bigint: true }).size : 0n
  let closed = false
  return {
    write(bytes) {
      const written = fs.writeSync(descriptor, bytes)
      position += BigInt(written)
      return BigInt(written)
    },
    tell() {
      return position
    },
    flush() {
      fs.fsyncSync(descriptor)
    },
    close() {
      if (!closed) fs.closeSync(descriptor)
      closed = true
    },
    get closed() {
      return closed
    },
  }
}

function local() {
  return {
    typeName: 'local',
    domain: localDomain,
    equals(other) {
      return other?.domain === localDomain
    },
    normalizePath(location) {
      return path.normalize(location)
    },
    fileInfo(location) {
      const stats = fs.statSync(location, {
        throwIfNoEntry: false,
        bigint: true,
      })
      if (stats === undefined) return { path: location, kind: 'not-found' }
      if (stats.isDirectory())
        return { path: location, kind: 'directory', mtimeNs: stats.mtimeNs }
      return {
        path: location,
        kind: 'file',
        size: stats.size,
        mtimeNs: stats.mtimeNs,
      }
    },
    list(selector) {
      let entries
      try {
        entries = fs.readdirSync(selector.baseDir, {
          recursive: selector.recursive,
          withFileTypes: true,
        })
      } catch (error) {
        if (selector.allowNotFound && error.code === 'ENOENT') return []
        throw error
      }
      return entries.map((entry) => {
        const full = path.join(entry.parentPath ?? entry.path, entry.name)
        return this.fileInfo(full)
      })
    },
    createDir(location, recursive) {
      fs.mkdirSync(location, { recursive })
    },
    deleteDir(location) {
      fs.rmdirSync(location)
    },
    deleteDirContents(location, missingDirOk) {
      if (!fs.existsSync(location)) {
        if (missingDirOk) return
        throw failure('NotFound', location)
      }
      for (const name of fs.readdirSync(location)) {
        fs.rmSync(path.join(location, name), { recursive: true })
      }
    },
    deleteRootDirContents() {
      throw failure('Unsupported', 'local root deletion')
    },
    deleteFile(location) {
      fs.unlinkSync(location)
    },
    copyFile(source, target) {
      fs.copyFileSync(source, target)
    },
    move(source, target) {
      fs.renameSync(source, target)
    },
    openInputFile: localReader,
    openInputStream: localReader,
    openOutputStream(location) {
      return localWriter(location, false)
    },
    openAppendStream(location) {
      return localWriter(location, true)
    },
  }
}

function scratch() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-fs-'))
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

test('bound facts keep the original handler, raw path, and safe URI', () => {
  const domain = {}
  const handler = memory(domain)
  const path = 'bucket/v=a%2Fb//x%25+y://z'
  const uri = 's3://key:secret@bucket/v=a%2Fb//x%25+y://z?session_token=hidden'
  const handle = IOBase.fromFs(handler, path, uri)

  assert.equal(handle.filesystem, handler)
  assert.equal(handle.path, path)
  assert.equal(handle.uri, uri)
  assert.ok(!handle.maskedUri.includes('secret'))
  assert.ok(!handle.maskedUri.includes('hidden'))
  assert.equal(
    handle.joinpath('literal%2F+name').path,
    `${path}/literal%2F+name`,
  )
  assert.equal(handle.parent.path, 'bucket/v=a%2Fb//x%25+y:/')

  assert.equal(handle.sameLocation(IOBase.fromFs(memory(domain), path)), true)
  assert.equal(handle.sameLocation(IOBase.fromFs(memory(), path)), false)
  assert.equal(
    handle.sameLocation(IOBase.fromFs(handler, `${path}/other`)),
    false,
  )
})

test('S3 URI forms resolve before Arrow JS reports its missing backend', () => {
  for (const uri of [
    's3://bucket/v=a%2Fb',
    's3a://bucket/key',
    's3://key:secret@bucket/key',
    's3://key:secret@minio:9000/bucket/key',
    's3://bucket/key?endpoint_override=minio%3A9000&scheme=http&region=eu-west-1',
    's3://bucket.s3.eu-west-1.amazonaws.com/key',
  ]) {
    assert.throws(
      () => IOBase.fromUri(uri),
      (error) => {
        assert.match(error.message, /does not support S3 filesystem URI/)
        assert.ok(!error.message.includes('secret'))
        return true
      },
    )
  }
})

test('file info preserves bigint size and nanosecond mtime', () => {
  const handler = memory()
  handler.fileInfo = (path) => ({
    path,
    kind: 'file',
    size: 9_007_199_254_740_993n,
    mtimeNs: 1_725_000_000_000_000_001n,
  })
  const info = IOBase.fromFs(handler, 'huge').info()
  assert.equal(info.size, 9_007_199_254_740_993n)
  assert.equal(info.mtimeNs, 1_725_000_000_000_000_001n)
})

test('handler streams are stateful, bounded, and explicitly closed', () => {
  const handler = memory()
  handler.files.set('source', Buffer.from('abcdefghij'))
  const calls = { info: 0, input: 0 }
  const fileInfo = handler.fileInfo
  handler.fileInfo = function countedInfo(...args) {
    calls.info += 1
    return fileInfo.apply(this, args)
  }
  const openInputStream = handler.openInputStream
  handler.openInputStream = function countedInput(...args) {
    calls.input += 1
    return openInputStream.apply(this, args)
  }
  const source = IOBase.fromFs(handler, 'source')

  const sequential = source.openInputStream()
  assert.equal(sequential.tell(), 0n)
  const chunks = []
  while (true) {
    const chunk = sequential.read(3n)
    if (chunk.length === 0) break
    chunks.push(chunk)
  }
  assert.equal(Buffer.concat(chunks).toString(), 'abcdefghij')
  assert.equal(sequential.tell(), 10n)
  sequential.close()
  sequential.close()
  assert.equal(sequential.closed, true)
  assert.deepEqual(calls, { info: 0, input: 1 })

  const random = source.openInputFile()
  assert.equal(random.readAt(5n, 3n).toString(), 'fgh')
  assert.equal(random.tell(), 0n)
  assert.equal(random.seek(2n), 2n)
  assert.equal(random.read(2n).toString(), 'cd')
  random.close()

  const target = IOBase.fromFs(handler, 'target')
  const output = target.openOutputStream(
    new Map([['content-type', 'application/test']]),
  )
  assert.equal(output.write(Buffer.from('abc')), 3n)
  assert.equal(output.write(Buffer.from('def')), 3n)
  output.flush()
  output.close()
  output.close()
  assert.equal(output.closed, true)
  assert.equal(handler.files.get('target').toString(), 'abcdef')
  assert.deepEqual(handler.metadata.get('target'), {
    'content-type': 'application/test',
  })
})

test('listings are sorted and selectors reach the handler intact', () => {
  const handler = memory()
  handler.files.set('root/z', Buffer.from('z'))
  handler.files.set('root/a', Buffer.from('a'))
  const selectors = []
  const list = handler.list
  handler.list = function recording(selector) {
    selectors.push({ ...selector })
    return list.call(this, selector)
  }
  const root = IOBase.fromFs(handler, 'root')
  assert.deepEqual(
    [...root.ls()].map((entry) => entry.path),
    ['root/a', 'root/z'],
  )
  assert.deepEqual(selectors[0], {
    baseDir: 'root',
    recursive: false,
    allowNotFound: true,
  })
})

test('same-filesystem copy and move use one native operation and no streams', () => {
  const handler = memory()
  handler.files.set('source', Buffer.from('payload'))
  const calls = { copy: 0, move: 0, input: 0, output: 0 }
  for (const [name, key] of [
    ['copyFile', 'copy'],
    ['move', 'move'],
    ['openInputStream', 'input'],
    ['openOutputStream', 'output'],
  ]) {
    const method = handler[name]
    handler[name] = function counted(...args) {
      calls[key] += 1
      return method.apply(this, args)
    }
  }

  const source = IOBase.fromFs(handler, 'source')
  const copied = IOBase.fromFs(handler, 'copied')
  assert.equal(source.copyInto(copied), 7n)
  assert.deepEqual(calls, { copy: 1, move: 0, input: 0, output: 0 })

  const moved = IOBase.fromFs(handler, 'moved')
  assert.equal(copied.moveInto(moved).sameLocation(moved), true)
  assert.deepEqual(calls, { copy: 1, move: 1, input: 0, output: 0 })
})

test('handler equality failures abort identity, copy, and move', () => {
  const handler = memory()
  handler.files.set('source', Buffer.from('kept'))
  handler.files.set('target', Buffer.from('original'))
  const calls = { copy: 0, move: 0, input: 0, output: 0 }
  for (const [name, key] of [
    ['copyFile', 'copy'],
    ['move', 'move'],
    ['openInputStream', 'input'],
    ['openOutputStream', 'output'],
  ]) {
    const method = handler[name]
    handler[name] = function counted(...args) {
      calls[key] += 1
      return method.apply(this, args)
    }
  }
  handler.equals = () => {
    throw failure('PermissionDenied', 'filesystem equality was refused')
  }
  const source = IOBase.fromFs(handler, 'source')
  const target = IOBase.fromFs(handler, 'target')
  const identicalPath = IOBase.fromFs(handler, 'source')
  for (const operation of [() => source.sameLocation(identicalPath)]) {
    assert.throws(operation, (error) => {
      assert.equal(error.code, 'PermissionDenied')
      return true
    })
  }
  for (const operation of [
    () => source.copyInto(target),
    () => source.moveInto(target),
  ]) {
    assert.throws(operation, (error) => {
      assert.equal(error.code, 'PermissionDenied')
      return true
    })
  }
  assert.deepEqual(calls, { copy: 0, move: 0, input: 0, output: 0 })
  assert.equal(handler.files.get('source').toString(), 'kept')
  assert.equal(handler.files.get('target').toString(), 'original')
})

test('cross-filesystem copy never publishes missing or partial input', () => {
  const sourceHandler = memory()
  const targetHandler = memory()
  targetHandler.files.set('target', Buffer.from('original'))
  let outputs = 0
  const output = targetHandler.openOutputStream
  targetHandler.openOutputStream = function counted(...args) {
    outputs += 1
    return output.apply(this, args)
  }

  assert.throws(
    () =>
      IOBase.fromFs(sourceHandler, 'missing').copyInto(
        IOBase.fromFs(targetHandler, 'target'),
      ),
    (error) => {
      assert.equal(error.code, 'NotFound')
      return true
    },
  )
  assert.equal(outputs, 0)
  assert.equal(targetHandler.files.get('target').toString(), 'original')

  sourceHandler.files.set('source', Buffer.from('partial payload'))
  sourceHandler.openInputStream = () => {
    let reads = 0
    return {
      read() {
        reads += 1
        if (reads === 1) return Buffer.from('partial')
        throw failure('Transport', 'source disconnected')
      },
      tell: () => 0n,
      close() {},
      get closed() {
        return false
      },
    }
  }
  assert.throws(
    () =>
      IOBase.fromFs(sourceHandler, 'source').copyInto(
        IOBase.fromFs(targetHandler, 'target'),
      ),
    (error) => {
      assert.equal(error.code, 'Transport')
      return true
    },
  )
  assert.equal(targetHandler.files.get('target').toString(), 'original')
  assert.equal(
    [...targetHandler.files.keys()].some((name) =>
      name.includes('.yggdryl-transfer-'),
    ),
    false,
  )
})

test('directory lifecycle operations remain distinct', () => {
  const handler = memory()
  handler.createDir('root', true)
  handler.files.set('root/child', Buffer.from('x'))
  const root = IOBase.fromFs(handler, 'root')

  assert.throws(() => root.deleteDir(), /DirectoryNotEmpty/)
  assert.throws(() => root.deleteFile(), /IsADirectory/)
  root.deleteDirContents(false)
  assert.equal(root.info().kind, 'directory')
  root.deleteDir()
  assert.equal(root.info().kind, 'not-found')

  handler.files.set('kept', Buffer.from('x'))
  assert.throws(
    () => root.deleteRootDirContents(),
    (error) => {
      assert.equal(error.code, 'Unsupported')
      return true
    },
  )
  assert.equal(handler.files.has('kept'), true)
  IOBase.fromFs(handler, '').deleteRootDirContents()
  assert.equal(handler.files.size, 0)
})

test('a handler-backed handle is an ordinary handle', () => {
  const handle = IOBase.fromFs(memory(), 'bucket/trades.parquet')

  // The file system's own name is the scheme its locations carry.
  assert.equal(handle.toString(), 'memory://bound/bucket/trades.parquet')
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
  const explicit = IOBase.fromFs(handler, 'bucket/inferred.bin')
  assert.equal(inferred.toString(), explicit.toString())

  inferred.writeText('same')
  inferred.flush()
  assert.equal(explicit.readText(), 'same')

  // A file system with nowhere to resolve, and a location with a file system
  // that is not one, are both refused by name.
  assert.throws(
    () => new IOBase(handler),
    /a path on the file system as the second argument/,
  )
  assert.throws(
    () => new IOBase('some/path', 'another'),
    /Arrow file system handler to resolve/,
  )
})

test('a handler missing a method is refused by that method', () => {
  const partial = { ...memory() }
  delete partial.openInputFile

  assert.throws(() => IOBase.fromFs(partial, 'bucket/key.bin'), /openInputFile/)
  assert.throws(() => IOBase.fromFs({}, 'bucket/key.bin'), /typeName/)
  // A method that is not callable is no method at all.
  assert.throws(
    () => IOBase.fromFs({ ...memory(), list: 'everything' }, 'bucket/key.bin'),
    /list/,
  )
})

test('bytes round trip through forwarding output and append streams', () => {
  const handler = memory()
  const handle = IOBase.fromFs(handler, 'bucket/streamed.bin')

  // A missing file starts with output and a write at its end uses append.
  handle.pwrite(0, Buffer.from('pend'))
  handle.pwrite(4, Buffer.from('ing'))
  assert.equal(handler.files.get('bucket/streamed.bin').toString(), 'pending')
  assert.equal(handle.readText(), 'pending')
  assert.ok(handle.isFile())

  handle.flush()
  assert.equal(handler.files.get('bucket/streamed.bin').toString(), 'pending')

  // The high-level write completes and closes its output stream.
  const overwrite = IOBase.fromFs(handler, 'bucket/overwrite.bin')
  overwrite.writeText('published')
  assert.equal(
    handler.files.get('bucket/overwrite.bin').toString(),
    'published',
  )

  // A backend that cannot support an arbitrary positional write says so.
  assert.throws(
    () => handle.pwrite(0, Buffer.from('PENDING')),
    /does not support/,
  )
  handle.writeText('PENDING')
  assert.equal(handle.readRangeBytes(0, 3).toString(), 'PEN')
  assert.equal(handle.appendBytes(Buffer.from('!')), 7)
  handle.flush()
  assert.equal(handler.files.get('bucket/streamed.bin').toString(), 'PENDING!')

  handle.unlink()
  handle.flush()
  assert.equal(handle.readBytes().length, 0)
})

test('a size crosses as an exact bigint', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  fs.writeFileSync(path.join(root, 'sized.bin'), 'symbol,price')

  // Both handlers preserve the filesystem's 64-bit size.
  assert.equal(IOBase.fromFs(local(), path.join(root, 'sized.bin')).size, 12)

  const handler = memory()
  handler.files.set('bucket/sized.bin', Buffer.from('symbol,price'))
  assert.equal(IOBase.fromFs(handler, 'bucket/sized.bin').size, 12)

  // A length no length can be is refused rather than truncated.
  const lying = {
    ...memory(),
    fileInfo: (location) => ({ path: location, kind: 'file', size: -1n }),
  }
  assert.throws(
    () => IOBase.fromFs(lying, 'bucket/key.bin').writeText('AAPL'),
    /unsigned 64-bit integer/,
  )
})

test('folders list, glob, and carry the file system', () => {
  const handler = memory()
  for (const year of ['2024', '2025']) {
    for (const month of ['01', '02']) {
      handler.files.set(
        `lake/year=${year}/month=${month}/part-0.parquet`,
        Buffer.from('PAR1'),
      )
      handler.files.set(
        `lake/year=${year}/month=${month}/notes.txt`,
        Buffer.from('notes'),
      )
    }
  }
  const lake = IOBase.fromFs(handler, 'lake')

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
  assert.equal(
    leaf.toString(),
    'memory://bound/lake/year=2024/month=01/part-0.parquet',
  )
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

  // The local handler maps `ENOENT` to an empty listing only because this
  // selector explicitly allows a missing base directory.
  const absent = IOBase.fromFs(
    local(),
    path.join(root, 'nowhere', 'absent.arrows'),
  )
  assert.ok(!absent.exists())
  assert.equal(absent.size, 0)
  assert.equal(absent.readBytes().length, 0)
  assert.deepEqual([...absent.ls(true)], [])
  // Removing what was never there is what was asked for.
  absent.unlink()

  // Parent creation is explicit; the output operation creates only the file.
  IOBase.fromFs(local(), path.join(root, 'deep', 'nested')).createDir(true)
  const nested = IOBase.fromFs(
    local(),
    path.join(root, 'deep', 'nested', 'trades.txt'),
  )
  nested.writeText('AAPL')
  nested.flush()
  assert.equal(
    fs.readFileSync(path.join(root, 'deep', 'nested', 'trades.txt'), 'utf8'),
    'AAPL',
  )
})

test('records round trip through the wrapper, in both encodings', (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  IOBase.fromFs(local(), path.join(root, 'lake')).createDir(true)

  for (const name of ['trades.arrows', 'trades.parquet']) {
    const handle = IOBase.fromFs(local(), path.join(root, 'lake', name))
    handle.overwriteArrowTable(trades())
    handle.flush()

    // What the wrapper published is a file any other reader can open, so the
    // local backend reads the same rows off the same bytes.
    const outside = new IOBase(path.join(root, 'lake', name))
    assert.deepEqual(outside.readBytes(), handle.readBytes())
    assert.equal(outside.readArrowReader().intoTable().numRows, 2)
    outside.close()

    const table = IOBase.fromFs(local(), path.join(root, 'lake', name))
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
      IOBase.fromFs(local(), path.join(root, 'lake', name))
        .readArrowReader()
        .intoTable().numRows,
      4,
    )
  }
})

test('records round trip over storage that is only a Map', () => {
  const handler = memory()
  const handle = IOBase.fromFs(handler, 'bucket/trades.arrows')

  handle.overwriteArrowTable(trades())
  handle.flush()

  // Every byte of that table went through the caller's own handler.
  assert.ok(handler.files.has('bucket/trades.arrows'))
  assert.equal(handle.readArrowField().dtype.length, 2)
  assert.equal(
    IOBase.fromFs(handler, 'bucket/trades.arrows').readArrowReader().intoTable()
      .numRows,
    2,
  )

  // Plain text reaches the same generic record path over a handler-backed
  // handle; each physical line is the binary body of one row.
  const lines = IOBase.fromFs(handler, 'bucket/rows.txt')
  lines.writeText('{"id":1}\n{"id":2}\n')
  lines.flush()
  assert.deepEqual(
    [...lines.readRecords(RecordOptions.from('text/plain'))].map((row) =>
      Buffer.from(row.body).toString(),
    ),
    ['{"id":1}', '{"id":2}'],
  )
})

test('handler-backed framed text resets at every leaf', () => {
  const handler = memory()
  handler.files.set(
    'bucket/logs/a.txt',
    Buffer.from('[INFO] first\ncontinuation from a'),
  )
  handler.files.set(
    'bucket/logs/b.txt',
    Buffer.from('leading fragment from b\n[WARN] second'),
  )

  const options = new TextOptions()
  options.framing = true
  options.rowheader = '^\\[(?<level>[A-Z]+)\\] '
  options.batchRowSize = 1

  const table = IOBase.fromFs(handler, 'bucket/logs')
    .readArrowReader(options)
    .intoTable()
  assert.deepEqual(
    table.schema.fields.map((field) => [field.name, field.nullable]),
    [
      ['url', false],
      ['body', false],
      ['level', true],
    ],
  )
  assert.deepEqual(
    [...table.getChild('body')].map((body) => Buffer.from(body).toString()),
    ['first\ncontinuation from a', 'leading fragment from b', 'second'],
  )
  assert.deepEqual([...table.getChild('level')], ['INFO', null, 'WARN'])
})

test('a handler that throws surfaces its own message', () => {
  const refusing = {
    ...memory(),
    fileInfo: (location) => ({ path: location, kind: 'file', size: 8n }),
    openInputStream() {
      throw new Error('the bucket refused the request: 403 Forbidden')
    },
    openInputFile() {
      throw new Error('the bucket refused the request: 403 Forbidden')
    },
  }
  const handle = IOBase.fromFs(refusing, 'bucket/key.bin')
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
    () => IOBase.fromFs(failing, 'bucket/key.bin').info(),
    /the credential chain is empty/,
  )

  // A kind the core has no name for is refused naming the vocabulary.
  const inventing = {
    ...memory(),
    fileInfo: (location) => ({ path: location, kind: 'blob' }),
  }
  assert.throws(
    () => IOBase.fromFs(inventing, 'bucket/key.bin').writeText('AAPL'),
    /file.*directory.*not-found/,
  )
})

test('filesystem error codes and sticky stream failures survive the boundary', () => {
  for (const code of [
    'NotFound',
    'PermissionDenied',
    'AlreadyExists',
    'NotADirectory',
    'IsADirectory',
    'DirectoryNotEmpty',
    'Unsupported',
    'Transport',
  ]) {
    const handler = memory()
    handler.deleteFile = () => {
      throw failure(code, `reported ${code}`)
    }
    assert.throws(
      () => IOBase.fromFs(handler, 'path').deleteFile(),
      (error) => {
        assert.equal(error.code, code)
        assert.match(error.message, new RegExp(code))
        return true
      },
    )
  }

  const handler = memory()
  let closes = 0
  handler.openOutputStream = () => ({
    write() {
      throw failure('PermissionDenied', 'write refused')
    },
    tell: () => 0n,
    flush() {},
    close() {
      closes += 1
    },
    get closed() {
      return closes !== 0
    },
  })
  const writer = IOBase.fromFs(handler, 'target').openOutputStream()
  for (const operation of [
    () => writer.write(Buffer.from('x')),
    () => writer.close(),
    () => writer.close(),
  ]) {
    assert.throws(operation, (error) => {
      assert.equal(error.code, 'PermissionDenied')
      return true
    })
  }
  assert.equal(closes, 1)
  assert.equal(writer.closed, true)
})

test('an Iceberg table lives on a file system a caller wrote', () => {
  const handler = memory()
  const warehouse = IOBase.fromFs(handler, 'warehouse/trades')
  const schema = iceberg.assignFieldIds(
    fields.struct(
      'row',
      [Field.from('id: int64'), Field.from('symbol: utf8')],
      {
        nullable: false,
      },
    ),
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

test('a scope closes an output stream', () => {
  const handler = memory()

  // The symbol is called directly because the `using` declaration is newer
  // than the oldest Node this suite runs on.
  {
    const handle = IOBase.fromFs(handler, 'bucket/scoped.bin')
    const output = handle.openOutputStream()
    output.write(Buffer.from('AAPL'))
    assert.equal(typeof output[Symbol.dispose], 'function')
    output[Symbol.dispose]()
  }

  assert.deepEqual(
    Buffer.from(handler.files.get('bucket/scoped.bin')).toString(),
    'AAPL',
  )
  assert.equal(IOBase.fromFs(handler, 'bucket/scoped.bin').readText(), 'AAPL')
})

test('open and close do not create an untouched handle', () => {
  const handler = memory()
  const handle = IOBase.fromFs(handler, 'bucket/absent.bin')

  // Opening a resource that does not exist yet caches nothing to publish.
  handle.open()
  handle.close()
  assert.equal(handler.files.has('bucket/absent.bin'), false)

  handle.writeText('later')
  assert.equal(handle.opened(), false)
  assert.equal(handle.closed(), true)
  handle.close()
  assert.equal(handle.opened(), false)
  assert.equal(handle.closed(), true)
  assert.equal(handle.readText(), 'later')
})

test('mkdir creates the container on the same file system', () => {
  const handler = memory()
  const handle = IOBase.fromFs(handler, 'bucket/lake')

  // A location does not say which backend it belongs to, so mkdir must not
  // quietly rebuild the handle on the local disk.
  handle.mkdir()
  const child = handle.joinpath(['part-0.bin'])
  child.writeText('AAPL')
  child.close()

  assert.equal(
    Buffer.from(handler.files.get('bucket/lake/part-0.bin')).toString(),
    'AAPL',
  )
  assert.equal(fs.existsSync('bucket/lake'), false)
})

test('a table hands back a root on its own file system', () => {
  const handler = memory()
  const warehouse = IOBase.fromFs(handler, 'warehouse/trades')
  const schema = iceberg.assignFieldIds(
    fields.struct(
      'row',
      [Field.from('id: int64'), Field.from('symbol: utf8')],
      {
        nullable: false,
      },
    ),
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
