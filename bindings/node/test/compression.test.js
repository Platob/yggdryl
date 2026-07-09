'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')

const { Gzip, Zstd } = yggdryl.compression

test('compression.Zstd round-trips + defaults', () => {
  const zstd = new Zstd()
  assert.equal(zstd.level, 3)
  assert.equal(zstd.name, 'zstd')
  const [lo, hi] = Zstd.levelRange()
  assert.ok(lo <= 3 && 3 <= hi)
  const original = Buffer.from('the quick brown fox '.repeat(200))
  const compressed = zstd.encodeByteArray(original)
  assert.ok(compressed.length < original.length)
  assert.deepEqual(zstd.decodeByteArray(compressed), original)
})

test('compression.Zstd value semantics', () => {
  assert.ok(new Zstd(9).equals(new Zstd(9)))
  assert.ok(!new Zstd(9).equals(new Zstd(3)))
  assert.ok(Zstd.deserializeBytes(new Zstd(9).serializeBytes()).equals(new Zstd(9)))
})

test('compression.Gzip round-trips bytes', () => {
  const gzip = new Gzip(6)
  const original = Buffer.from('the quick brown fox jumps over the lazy dog'.repeat(16))
  const compressed = gzip.encodeByteArray(original)
  assert.ok(compressed.length < original.length)
  assert.deepEqual(gzip.decodeByteArray(compressed), original)
})

test('compression.Gzip defaults to level 6', () => {
  const gzip = new Gzip()
  assert.equal(gzip.level, 6)
  assert.equal(gzip.name, 'gzip')
})

test('compression.Gzip rejects an invalid level', () => {
  assert.throws(() => new Gzip(10))
})

test('compression.Gzip rejects a corrupt stream', () => {
  assert.throws(() => new Gzip().decodeByteArray(Buffer.from('not a gzip stream')))
})

test('compression.Gzip round-trips through bytes', () => {
  const gzip = new Gzip(9)
  const restored = Gzip.deserializeBytes(gzip.serializeBytes())
  assert.equal(restored.level, 9)
})

test('compression.Gzip equality and hashing', () => {
  assert.ok(new Gzip(6).equals(new Gzip()))
  assert.ok(!new Gzip(6).equals(new Gzip(9)))
  assert.equal(new Gzip(6).hashCode(), new Gzip().hashCode())
  assert.notEqual(new Gzip(6).hashCode(), new Gzip(9).hashCode())
})
