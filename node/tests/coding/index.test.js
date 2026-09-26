'use strict'

// Pins node/src/coding/mod.rs: the whole-buffer content codings, grouped by
// the loader into the `gzip`, `zlib` and `zstd` namespaces.

const assert = require('node:assert/strict')
const test = require('node:test')
const zlibNative = require('node:zlib')

const { gzip, zlib, zstd } = require('yggdryl')

const payload = Buffer.from('{"id":1}\n'.repeat(512))

test('the byte codings round-trip and read node:zlib output', () => {
  assert.deepEqual(gzip.loads(gzip.dumps(payload)), payload)
  assert.deepEqual(gzip.loads(zlibNative.gzipSync(payload)), payload)
  assert.deepEqual(zlibNative.gunzipSync(gzip.dumps(payload, 9)), payload)
  assert.deepEqual(zlib.loads(zlib.dumps(payload)), payload)
  assert.deepEqual(zlib.loads(zlibNative.deflateSync(payload)), payload)
  assert.deepEqual(zstd.loads(zstd.dumps(payload, 9)), payload)
})

test('raw DEFLATE round-trips, reads node:zlib, and shares no framing with zlib', () => {
  assert.deepEqual(zlib.loadsRaw(zlib.dumpsRaw(payload)), payload)
  assert.deepEqual(zlib.loadsRaw(zlibNative.deflateRawSync(payload)), payload)
  assert.deepEqual(zlibNative.inflateRawSync(zlib.dumpsRaw(payload, 9)), payload)
  // The raw output is the framed output without the two-byte header and the
  // four-byte checksum, which is the whole of the difference.
  assert.equal(zlib.dumpsRaw(payload).length + 6, zlib.dumps(payload).length)

  // Nothing in unframed bytes says which framing they are, so the pair is
  // named rather than sniffed - and each half refuses the other's output
  // instead of decoding it into something plausible.
  assert.throws(() => zlib.loads(zlib.dumpsRaw(payload)), /deflate/)
  assert.throws(() => zlib.loadsRaw(zlib.dumps(payload)), /deflate/)
})

test('each namespace is exactly its halves, frozen', () => {
  assert.deepEqual(Object.keys(gzip).sort(), ['dumps', 'loads'])
  assert.deepEqual(Object.keys(zstd).sort(), ['dumps', 'loads'])
  assert.deepEqual(Object.keys(zlib).sort(), ['dumps', 'dumpsRaw', 'loads', 'loadsRaw'])
  for (const coding of [gzip, zlib, zstd]) assert.ok(Object.isFrozen(coding))
})

test('any Uint8Array is content, and a level past the scale clamps to its top', () => {
  const view = new Uint8Array(payload.buffer, payload.byteOffset, payload.byteLength)
  for (const coding of [gzip, zlib, zstd]) {
    const encoded = coding.dumps(view)
    assert.ok(Buffer.isBuffer(encoded))
    assert.deepEqual(coding.loads(new Uint8Array(encoded)), payload)
    // The level is the shared 0-9 scale: past it is its top, and an absent
    // or null level is the coding's default.
    assert.deepEqual(coding.dumps(payload, 12), coding.dumps(payload, 9))
    assert.deepEqual(coding.dumps(payload, null), coding.dumps(payload))
    assert.deepEqual(coding.loads(coding.dumps(payload, 0)), payload)
    assert.throws(() => coding.dumps(payload, -1))
  }
  assert.throws(() => gzip.loads(payload), /gzip|header|invalid/i)
})
