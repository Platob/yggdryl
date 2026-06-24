// Tests for the yggdryl Node.js extension's MediaType.
// Build first with `npm run build`, then run `node --test`.

const { test } = require('node:test')
const assert = require('node:assert')
const { Uri, Url, MediaType } = require('..')

test('mime parse and components', () => {
  const m = new MediaType('application/json')
  assert.strictEqual(m.mime, 'application/json')
  assert.strictEqual(m.type, 'application')
  assert.strictEqual(m.subtype, 'json')
  assert.strictEqual(m.toString(), 'application/json')
  assert.ok(m.isKnown)
  // Parameters are dropped; case is normalised.
  assert.strictEqual(new MediaType('Text/HTML; charset=utf-8').subtype, 'html')
})

test('from extension', () => {
  assert.strictEqual(MediaType.fromExtension('parquet').mime, 'application/vnd.apache.parquet')
  assert.strictEqual(MediaType.fromExtension('.GZ').mime, 'application/gzip')
  assert.strictEqual(MediaType.fromExtension('png').subtype, 'png')
  assert.strictEqual(MediaType.fromExtension('nope'), null)
})

test('from magic bytes', () => {
  assert.strictEqual(MediaType.fromMagic(Buffer.from('PAR1\x15\x04')).mime, 'application/vnd.apache.parquet')
  assert.strictEqual(MediaType.fromMagic(Buffer.from('ARROW1\x00\x00')).mime, 'application/vnd.apache.arrow.file')
  assert.strictEqual(MediaType.fromMagic(Buffer.from('PK\x03\x04\x14')).subtype, 'zip')
  assert.strictEqual(MediaType.fromMagic(Buffer.from([0x1f, 0x8b, 0x08, 0x00])).mime, 'application/gzip')
  assert.strictEqual(MediaType.fromMagic(Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])).subtype, 'png')
  assert.strictEqual(MediaType.fromMagic(Buffer.from('not magic')), null)
})

test('unknown other', () => {
  const m = new MediaType('application/x-custom')
  assert.ok(!m.isKnown)
  assert.strictEqual(m.subtype, 'x-custom')
  assert.strictEqual(m.extension, null)
  assert.deepStrictEqual(m.extensions, [])
})

test('invalid mime throws', () => {
  assert.throws(() => new MediaType('notamime'))
  assert.strictEqual(new MediaType('notamime', false).mime, 'notamime')
})

test('to/from mapping', () => {
  const m = new MediaType('image/svg+xml')
  assert.deepStrictEqual(m.toMapping(), { type: 'image', subtype: 'svg+xml' })
  assert.strictEqual(MediaType.fromMapping({ type: 'text', subtype: 'csv' }).mime, 'text/csv')
})

test('extensions and equality', () => {
  const jpeg = MediaType.fromExtension('jpg')
  assert.deepStrictEqual(jpeg.extensions, ['jpg', 'jpeg'])
  assert.strictEqual(jpeg.extension, 'jpg')
  assert.ok(jpeg.equals(new MediaType('image/jpeg')))
})

test('from path', () => {
  assert.strictEqual(MediaType.fromPath('/data/sales.parquet').mime, 'application/vnd.apache.parquet')
  assert.strictEqual(MediaType.fromPath('archive.tar.gz').mime, 'application/gzip')
  assert.strictEqual(MediaType.fromPath('/usr/bin/env'), null)
})

test('uri/url media type', () => {
  assert.strictEqual(new Uri('https://h/a/file.json').mediaType().mime, 'application/json')
  assert.strictEqual(new Url('https://h/data/sales.parquet').mediaType().subtype, 'vnd.apache.parquet')
  assert.strictEqual(new Uri('file:/dump/archive.tar.gz').mediaType().mime, 'application/gzip')
  assert.strictEqual(new Uri('https://h/page').mediaType(), null)
})
