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

    assert.deepStrictEqual(io.read(5), Buffer.from('hello'))
    assert.strictEqual(io.tell(), 5)
    // Positioned read leaves the cursor put.
    assert.deepStrictEqual(io.readAt(6, 5), Buffer.from('world'))
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

test('missing path throws', () => {
  assert.throws(() => new LocalPath('/no/such/yggdryl/path'))
})
