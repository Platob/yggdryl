// Tests for the yggdryl Node.js extension's LocalPath and IoStats.
// Build first with `npm run build`, then run `node --test`.

const { test } = require('node:test')
const assert = require('node:assert')
const os = require('node:os')
const path = require('node:path')
const fs = require('node:fs')
const { LocalPath } = require('..')

function temp(name, data) {
  const p = path.join(os.tmpdir(), `yggdryl_node_${process.pid}_${name}`)
  LocalPath.write(p, Buffer.from(data))
  return p
}

test('open, read, seek and random access', () => {
  const p = temp('read', 'hello world')
  try {
    const io = new LocalPath(p)
    assert.strictEqual(io.location, p)
    assert.ok(io.exists())
    assert.strictEqual(io.length, 11)
    assert.strictEqual(io.url.scheme, 'file')

    assert.deepStrictEqual(io.read(5), Buffer.from('hello'))
    assert.strictEqual(io.tell(), 5)
    // Positional pread leaves the cursor put (size, offset, whence=0).
    assert.deepStrictEqual(io.pread(5, 6), Buffer.from('world'))
    assert.strictEqual(io.tell(), 5)
    assert.deepStrictEqual(io.getValue(), Buffer.from('hello world'))
    io.seek(0)
    assert.deepStrictEqual(io.read(), Buffer.from('hello world'))
  } finally {
    fs.rmSync(p)
  }
})

test('stats', () => {
  const p = temp('stats', '0123456789')
  try {
    const stats = new LocalPath(p).stats()
    assert.strictEqual(stats.size, 10)
    assert.ok(stats.mtime > 0)
    assert.strictEqual(stats.contentType, null)
  } finally {
    fs.rmSync(p)
  }
})

test('media type inferred from extension', () => {
  const p = temp('media.csv', 'a,b,c\n1,2,3\n')
  try {
    const io = new LocalPath(p)
    const media = io.mediaType()
    assert.notStrictEqual(media, null)
    assert.strictEqual(media.first.subtype, 'csv')
    assert.notStrictEqual(io.stats().mediaType, null)
  } finally {
    fs.rmSync(p)
  }
})

test('stat classifies kind', () => {
  const missing = path.join(os.tmpdir(), `yggdryl_node_${process.pid}_nope`)
  assert.strictEqual(LocalPath.stat(missing).kind, 'missing')
  assert.strictEqual(LocalPath.stat(missing).exists, false)

  const f = temp('kind_file', 'hello')
  const d = path.join(os.tmpdir(), `yggdryl_node_${process.pid}_kind_dir`)
  fs.mkdirSync(d, { recursive: true })
  try {
    const fileStats = LocalPath.stat(f)
    assert.strictEqual(fileStats.kind, 'file')
    assert.ok(fileStats.isFile && fileStats.exists)
    assert.strictEqual(fileStats.size, 5)

    const dirStats = LocalPath.stat(d)
    assert.strictEqual(dirStats.kind, 'directory')
    assert.ok(dirStats.isDir)
    // An opened file reports kind "file" too.
    assert.strictEqual(new LocalPath(f).stats().kind, 'file')
  } finally {
    fs.rmSync(f)
    fs.rmSync(d, { recursive: true, force: true })
  }
})

test('missing path throws', () => {
  assert.throws(() => new LocalPath('/no/such/yggdryl/path'))
})

test('write auto-creates missing parent dirs', () => {
  const base = path.join(os.tmpdir(), `yggdryl_node_${process.pid}_autodir`)
  const nested = path.join(base, 'a', 'b', 'c.bin')
  try {
    // The parent directories do not exist yet; the write creates them.
    LocalPath.write(nested, Buffer.from('deep'))
    assert.deepStrictEqual(new LocalPath(nested).read(), Buffer.from('deep'))
  } finally {
    fs.rmSync(base, { recursive: true, force: true })
  }
})
