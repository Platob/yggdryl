'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { MediaType } = yggdryl.mediatype
const { MimeType } = yggdryl.mimetype

test('the mediatype namespace exposes MediaType', () => {
  assert.equal(typeof MediaType, 'function')
})

test('parse reads a comma-separated mime list', () => {
  const media = MediaType.parse('application/json, text/html')
  assert.equal(media.len(), 2)
  assert.ok(!media.isEmpty())
  assert.equal(media.primary().essence, 'application/json') // primary is the first
  assert.deepEqual(media.essences(), ['application/json', 'text/html'])
  assert.equal(media.toString(), 'application/json, text/html')

  // Empty items are skipped; a bad item is a guided error.
  assert.equal(MediaType.parse('application/json, , text/html').len(), 2)
  assert.throws(() => MediaType.parse('application/json, notamime'), /type\/subtype/)
})

test('fromExtensions builds the layered stack of a multi-extension name', () => {
  const tgz = MediaType.fromExtensions(['tar', 'gz'])
  assert.deepEqual(tgz.essences(), ['application/x-tar', 'application/gzip'])
  assert.equal(tgz.primary().essence, 'application/x-tar')

  // Unknown extensions are skipped.
  assert.deepEqual(MediaType.fromExtensions(['zzz', 'json']).essences(), ['application/json'])
  assert.ok(MediaType.fromExtensions(['zzz']).isEmpty())
})

test('of / constructor build from MimeType values; empty by default', () => {
  const json = MimeType.parse('application/json')
  assert.deepEqual(MediaType.of(json).essences(), ['application/json'])

  const built = new MediaType([json, MimeType.parse('application/gzip')])
  assert.deepEqual(built.essences(), ['application/json', 'application/gzip'])

  const empty = new MediaType()
  assert.equal(empty.len(), 0)
  assert.ok(empty.isEmpty())
  assert.equal(empty.primary(), null) // the one justified null
})

test('push appends; contains is case-insensitive', () => {
  const media = MediaType.of(MimeType.parse('application/json'))
  media.push(MimeType.parse('text/html'))
  assert.deepEqual(media.essences(), ['application/json', 'text/html'])
  assert.ok(media.contains('TEXT/HTML')) // case-insensitive
  assert.ok(!media.contains('image/png'))

  // types() returns the MimeType values, primary first.
  assert.ok(media.types().every((t) => t instanceof MimeType))
  assert.equal(media.types()[0].essence, 'application/json')
})

test('copy is an independent value', () => {
  const base = MediaType.parse('application/json')
  const dup = base.copy()
  dup.push(MimeType.parse('text/html'))
  assert.equal(base.len(), 1) // original untouched
  assert.equal(dup.len(), 2)
  assert.ok(base.copy().equals(base))
})

test('serializeBytes / deserializeBytes round-trip the comma-joined essences', () => {
  const media = MediaType.parse('application/x-tar, application/gzip')
  const raw = media.serializeBytes()
  assert.ok(Buffer.isBuffer(raw))
  assert.equal(raw.toString('utf8'), 'application/x-tar, application/gzip')
  const back = MediaType.deserializeBytes(raw)
  assert.ok(back.equals(media))
  assert.equal(back.hashCode(), media.hashCode())

  // Non-UTF-8 / bad-item bytes are a guided error.
  assert.throws(() => MediaType.deserializeBytes(Buffer.from([0xff, 0xfe])), /UTF-8/)
  assert.throws(() => MediaType.deserializeBytes(Buffer.from('a/b, notamime')), /type\/subtype/)
})

test('value semantics: equal lists hash equal', () => {
  const a = MediaType.parse('application/json, text/html')
  const b = new MediaType([MimeType.parse('application/json'), MimeType.parse('text/html')])
  assert.ok(a.equals(b))
  assert.equal(a.hashCode(), b.hashCode())
  assert.ok(!a.equals(MediaType.parse('application/json')))
})
