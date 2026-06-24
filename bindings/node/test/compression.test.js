// Tests for Compression — naming, parsing and round-trips.
// Build first with `npm run build`, then run `node --test`.

const { test } = require('node:test')
const assert = require('node:assert')
const { Compression } = require('..')

test('parses names and extensions', () => {
  assert.strictEqual(new Compression('gzip').name, 'gzip')
  assert.strictEqual(Compression.fromStr('GZ').name, 'gzip')
  assert.strictEqual(new Compression('zst').name, 'zstd')
  assert.strictEqual(new Compression(' snappy ').name, 'snappy')
  assert.strictEqual(new Compression('store').name, 'none')

  assert.strictEqual(new Compression('gzip').extension, 'gz')
  assert.strictEqual(new Compression('none').extension, null)
  assert.strictEqual(Compression.fromExtension('.zst').name, 'zstd')
  assert.strictEqual(Compression.fromExtension('txt'), null)

  assert.throws(() => new Compression('lzo'))
})

test('none is identity', () => {
  const codec = new Compression('none')
  assert.strictEqual(codec.isAvailable, true)
  const payload = Buffer.from('the quick brown fox')
  assert.deepStrictEqual(codec.compress(payload), payload)
  assert.deepStrictEqual(codec.decompress(payload), payload)
})

for (const name of ['gzip', 'zstd', 'snappy']) {
  test(`round-trips ${name}`, () => {
    const codec = new Compression(name)
    assert.strictEqual(codec.isAvailable, true) // the addon enables all three backends
    const payload = Buffer.from(Array.from({ length: 4096 }, (_, i) => i % 251))
    const packed = codec.compress(payload)
    assert.deepStrictEqual(codec.decompress(packed), payload)
  })
}

test('toString', () => {
  assert.strictEqual(new Compression('zstd').toString(), 'zstd')
})
