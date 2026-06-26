// Tests for the yggdryl Node.js extension's LocalPath and IoStats.
// Build first with `npm run build`, then run `node --test`.

const { test } = require('node:test')
const assert = require('node:assert')
const os = require('node:os')
const path = require('node:path')
const fs = require('node:fs')
const { LocalPath, open } = require('..')

function temp(name, data) {
  const p = path.join(os.tmpdir(), `yggdryl_node_${process.pid}_${name}`)
  new LocalPath(p).write(Buffer.from(data))
  return p
}

test('open factory dispatches on scheme', () => {
  const p = temp('factory', 'by-the-factory')
  // A bare path (and a file:// URL) resolves to a LocalPath handle.
  assert.deepStrictEqual(open(p).getValue(), Buffer.from('by-the-factory'))
  assert.deepStrictEqual(open('file://' + p).getValue(), Buffer.from('by-the-factory'))
  // A remote scheme is served by HttpSession, not this factory.
  assert.throws(() => open('https://example.com/x'))
})

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

test('stats classify kind', () => {
  // Missing — the instance is still constructible, with kind "missing".
  const missing = path.join(os.tmpdir(), `yggdryl_node_${process.pid}_nope`)
  assert.strictEqual(new LocalPath(missing).stats().kind, 'missing')
  assert.strictEqual(new LocalPath(missing).stats().exists, false)
  assert.strictEqual(new LocalPath(missing).exists(), false)

  const f = temp('kind_file', 'hello')
  const d = path.join(os.tmpdir(), `yggdryl_node_${process.pid}_kind_dir`)
  fs.mkdirSync(d, { recursive: true })
  try {
    const fileStats = new LocalPath(f).stats()
    assert.strictEqual(fileStats.kind, 'file')
    assert.ok(fileStats.isFile && fileStats.exists)
    assert.strictEqual(fileStats.size, 5)

    const dirStats = new LocalPath(d).stats()
    assert.strictEqual(dirStats.kind, 'directory')
    assert.ok(dirStats.isDir)
  } finally {
    fs.rmSync(f)
    fs.rmSync(d, { recursive: true, force: true })
  }
})

test('write auto-creates missing parent dirs', () => {
  const base = path.join(os.tmpdir(), `yggdryl_node_${process.pid}_autodir`)
  const nested = path.join(base, 'a', 'b', 'c.bin')
  try {
    // The parent directories do not exist yet; the write creates them.
    new LocalPath(nested).write(Buffer.from('deep'))
    assert.deepStrictEqual(new LocalPath(nested).read(), Buffer.from('deep'))
  } finally {
    fs.rmSync(base, { recursive: true, force: true })
  }
})

test('cachedStats get/set', () => {
  const { IoStats } = require('..')
  const p = path.join(os.tmpdir(), `yggdryl_node_${process.pid}_cached.bin`)
  new LocalPath(p).write(Buffer.from('hello'))
  try {
    const lp = new LocalPath(p)
    // Held since construction -> always present for a path.
    assert.strictEqual(lp.cachedStats().size, 5)
    lp.setStats(new IoStats(7, 'file', undefined, 'text/plain'))
    assert.strictEqual(lp.cachedStats().contentType, 'text/plain')
    assert.strictEqual(lp.stats().contentType, 'text/plain')
  } finally {
    fs.rmSync(p, { force: true })
  }
})
