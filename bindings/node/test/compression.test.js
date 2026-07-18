'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { Gzip, Zlib, Zstd, Lzma, codecFor } = yggdryl.compression
const { MimeType } = yggdryl.mimetype

// A compressible payload — long and repetitive, so every codec shrinks it.
const PAYLOAD = Buffer.from('yggdryl the world tree — one byte layer, many sources. '.repeat(400))

// Each codec, with its expected mime essence + short name.
const CODECS = [
  { name: 'Gzip', Class: Gzip, essence: 'application/gzip', shortName: 'gzip' },
  { name: 'Zlib', Class: Zlib, essence: 'application/zlib', shortName: 'zlib' },
  { name: 'Zstd', Class: Zstd, essence: 'application/zstd', shortName: 'zstd' },
  { name: 'Lzma', Class: Lzma, essence: 'application/x-xz', shortName: 'xz' },
]

test('the compression namespace exposes the four codecs + codecFor', () => {
  assert.equal(typeof Gzip, 'function')
  assert.equal(typeof Zlib, 'function')
  assert.equal(typeof Zstd, 'function')
  assert.equal(typeof Lzma, 'function')
  assert.equal(typeof codecFor, 'function')
})

for (const { name, Class, essence, shortName } of CODECS) {
  test(`${name}: essence / name / toString getters`, () => {
    const codec = new Class()
    assert.equal(codec.essence, essence)
    assert.equal(codec.name, shortName)
    assert.equal(codec.toString(), shortName)
  })

  test(`${name}: round-trips and shrinks a compressible payload`, () => {
    const codec = new Class()
    const packed = codec.compress(PAYLOAD)
    assert.ok(Buffer.isBuffer(packed))
    assert.ok(packed.length < PAYLOAD.length, 'compressed output is smaller')
    assert.deepEqual(codec.decompress(packed), PAYLOAD) // exact inverse
  })

  test(`${name}: an explicit level still round-trips`, () => {
    // gzip/zlib/xz take 0..9, zstd 1..22 — a mid level is valid for all.
    const codec = new Class(4)
    assert.deepEqual(codec.decompress(codec.compress(PAYLOAD)), PAYLOAD)
    // A negative level is clamped by the core rather than throwing.
    assert.deepEqual(new Class(-5).decompress(new Class(-5).compress(PAYLOAD)), PAYLOAD)
  })

  test(`${name}: an empty input round-trips to empty`, () => {
    const codec = new Class()
    const packed = codec.compress(Buffer.alloc(0))
    assert.equal(codec.decompress(packed).length, 0)
  })

  test(`${name}: corrupt input throws a guided Error`, () => {
    const codec = new Class()
    assert.throws(() => codec.decompress(Buffer.from([0, 1, 2, 3, 4, 5, 6, 7, 8, 9])), /./)
  })
}

test('cross-codec: one codec cannot decode another codec stream', () => {
  const gz = new Gzip().compress(PAYLOAD)
  assert.throws(() => new Zstd().decompress(gz)) // wrong magic -> guided error
})

test('codecFor resolves by mime essence string', () => {
  assert.ok(codecFor('application/gzip') instanceof Gzip)
  assert.ok(codecFor('application/zlib') instanceof Zlib)
  assert.ok(codecFor('application/zstd') instanceof Zstd)
  assert.ok(codecFor('application/x-xz') instanceof Lzma)
  assert.ok(codecFor('application/x-lzma') instanceof Lzma) // the lzma-alone alias essence

  // A non-compression essence is a justified null.
  assert.equal(codecFor('application/json'), null)
  assert.equal(codecFor('text/plain'), null)
  assert.equal(codecFor('not-a-mime'), null) // unparseable -> null, not a throw
})

test('codecFor resolves from a MimeType', () => {
  assert.ok(codecFor(MimeType.fromExtension('gz')) instanceof Gzip)
  assert.ok(codecFor(MimeType.fromExtension('zst')) instanceof Zstd)
  assert.ok(codecFor(MimeType.parse('application/x-xz')) instanceof Lzma)
  assert.equal(codecFor(MimeType.parse('image/png')), null)
})

test('a codecFor codec compresses interchangeably with a direct one', () => {
  const resolved = codecFor('application/gzip')
  const packed = resolved.compress(PAYLOAD)
  assert.deepEqual(new Gzip().decompress(packed), PAYLOAD) // same codec, both directions
})
