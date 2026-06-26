// Tests for the yggdryl Node.js extension's MimeType and MediaType.
// Build first with `npm run build`, then run `node --test`.

const { test } = require('node:test')
const assert = require('node:assert')
const { Uri, Url, MimeType, MediaType } = require('..')

test('mime parse and components', () => {
  const m = new MimeType('application/json')
  assert.strictEqual(m.mime, 'application/json')
  assert.strictEqual(m.type, 'application')
  assert.strictEqual(m.subtype, 'json')
  assert.strictEqual(m.toString(), 'application/json')
  assert.ok(m.isKnown)
  assert.strictEqual(new MimeType('Text/HTML; charset=utf-8').subtype, 'html')
})

test('mime from extension', () => {
  assert.strictEqual(MimeType.fromExtension('parquet').mime, 'application/vnd.apache.parquet')
  assert.strictEqual(MimeType.fromExtension('.GZ').mime, 'application/gzip')
  assert.strictEqual(MimeType.fromExtension('png').subtype, 'png')
  assert.strictEqual(MimeType.fromExtension('nope'), null)
})

test('mime from magic bytes', () => {
  assert.strictEqual(MimeType.fromMagic(Buffer.from('PAR1\x15\x04')).mime, 'application/vnd.apache.parquet')
  assert.strictEqual(MimeType.fromMagic(Buffer.from('ARROW1\x00\x00')).mime, 'application/vnd.apache.arrow.file')
  assert.strictEqual(MimeType.fromMagic(Buffer.from('PK\x03\x04\x14')).subtype, 'zip')
  assert.strictEqual(MimeType.fromMagic(Buffer.from([0x1f, 0x8b, 0x08, 0x00])).mime, 'application/gzip')
  assert.strictEqual(MimeType.fromMagic(Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])).subtype, 'png')
  assert.strictEqual(MimeType.fromMagic(Buffer.from('not magic')), null)
})

test('mime unknown other', () => {
  const m = new MimeType('application/x-custom')
  assert.ok(!m.isKnown)
  assert.strictEqual(m.subtype, 'x-custom')
  assert.strictEqual(m.extension, null)
  assert.deepStrictEqual(m.extensions, [])
})

test('mime invalid throws', () => {
  assert.throws(() => new MimeType('notamime'))
})

test('from str short names', () => {
  assert.ok(new MimeType('json').equals(new MimeType('application/json')))
  assert.strictEqual(new MimeType('gzip').mime, 'application/gzip')
  assert.strictEqual(new MimeType('zstd').mime, 'application/zstd')
  assert.throws(() => new MimeType('nope'))
  assert.deepStrictEqual(MediaType.fromStr('gzip').types.map((t) => t.mime), ['application/gzip'])
  assert.deepStrictEqual(MediaType.fromStr('nope').types, [])
})

test('mime to/from mapping and equality', () => {
  const m = new MimeType('image/svg+xml')
  assert.deepStrictEqual(m.toMapping(), { type: 'image', subtype: 'svg+xml' })
  assert.strictEqual(MimeType.fromMapping({ type: 'text', subtype: 'csv' }).mime, 'text/csv')
  const jpeg = MimeType.fromExtension('jpg')
  assert.deepStrictEqual(jpeg.extensions, ['jpg', 'jpeg'])
  assert.ok(jpeg.equals(new MimeType('image/jpeg')))
})

test('registry add and remove', () => {
  assert.strictEqual(MimeType.fromExtension('ygg'), null)
  try {
    MimeType.register('application/x-yggdryl', ['ygg'], [Buffer.from('YGG1')])
    const m = MimeType.fromExtension('ygg')
    assert.strictEqual(m.mime, 'application/x-yggdryl')
    assert.deepStrictEqual(m.extensions, ['ygg'])
    assert.strictEqual(MimeType.fromMagic(Buffer.from('YGG1\x00')).mime, 'application/x-yggdryl')
    assert.ok(MimeType.unregister('application/x-yggdryl'))
    assert.strictEqual(MimeType.fromExtension('ygg'), null)
    assert.ok(!MimeType.unregister('application/x-yggdryl'))
  } finally {
    MimeType.unregister('application/x-yggdryl')
  }
})

test('media type is ordered stack', () => {
  const stack = MediaType.fromPath('data.csv.gz')
  assert.deepStrictEqual(stack.types.map((t) => t.mime), ['text/csv', 'application/gzip'])
  assert.strictEqual(stack.first.mime, 'text/csv')
  assert.strictEqual(stack.last.mime, 'application/gzip')
  assert.strictEqual(stack.length, 2)
  assert.strictEqual(stack.toString(), 'csv.gz')
})

test('tgz compound and newly-added mime types', () => {
  // `.tgz` is tar+gzip — the same stack as `.tar.gz`.
  const tgz = MediaType.fromPath('app.tgz').types.map((t) => t.mime)
  assert.deepStrictEqual(tgz, ['application/x-tar', 'application/gzip'])
  assert.deepStrictEqual(tgz, MediaType.fromPath('app.tar.gz').types.map((t) => t.mime))
  // A selection of newly-added common MIME types resolve by extension / magic.
  assert.strictEqual(MimeType.fromExtension('yaml').mime, 'application/yaml')
  assert.strictEqual(MimeType.fromExtension('toml').mime, 'application/toml')
  assert.strictEqual(MimeType.fromExtension('avif').mime, 'image/avif')
  assert.strictEqual(MimeType.fromExtension('mkv').mime, 'video/x-matroska')
  assert.strictEqual(MimeType.fromMagic(Buffer.from('\xfd7zXZ\x00\x00', 'binary')).mime, 'application/x-xz')
})

test('media type explicit construction', () => {
  const stack = new MediaType([new MimeType('text/csv'), new MimeType('application/gzip')])
  assert.ok(stack.equals(MediaType.fromPath('x.csv.gz')))
  assert.ok(MediaType.fromPath('/usr/bin/env').isEmpty)
  assert.strictEqual(new MediaType([]).length, 0)
})

test('convenient from constructors', () => {
  assert.ok(MimeType.fromParts('text', 'csv').equals(new MimeType('text/csv')))
  assert.strictEqual(MimeType.fromParts('application', 'x-foo').mime, 'application/x-foo')
  assert.strictEqual(MimeType.fromPath('data.csv.gz').mime, 'application/gzip')
  assert.strictEqual(MimeType.fromPath('notes'), null)
  assert.deepStrictEqual(MediaType.fromExtension('json').types.map((t) => t.mime), ['application/json'])
  assert.deepStrictEqual(
    MediaType.fromExtensions(['csv', 'nope', 'gz']).types.map((t) => t.mime),
    ['text/csv', 'application/gzip'],
  )
  // toMapping/fromMapping round-trip via the `types` key (MIME list).
  const stack = MediaType.fromPath('a/b.csv.gz')
  assert.deepStrictEqual(stack.toMapping(), { types: 'text/csv,application/gzip' })
  assert.ok(MediaType.fromMapping(stack.toMapping()).equals(stack))
  assert.ok(MediaType.fromMapping({ types: 'text/csv,application/gzip' }).equals(stack))
})

test('default octet-stream fallback', () => {
  assert.strictEqual(MimeType.default().mime, 'application/octet-stream')
  assert.deepStrictEqual(MediaType.default().types.map((t) => t.mime), ['application/octet-stream'])
  // Conventional fallback for failed inference.
  assert.strictEqual((MimeType.fromPath('notes') ?? MimeType.default()).mime, 'application/octet-stream')
})

test('uri/url media type', () => {
  assert.deepStrictEqual(new Uri('https://h/a/file.json').mediaType().types.map((t) => t.mime), ['application/json'])
  const url = new Url('https://h/dump/archive.tar.gz')
  assert.deepStrictEqual(url.mediaType().types.map((t) => t.mime), ['application/x-tar', 'application/gzip'])
  assert.strictEqual(url.mimeType().mime, 'application/gzip')
  assert.strictEqual(new Uri('https://h/page').mediaType(), null)
  assert.strictEqual(new Uri('https://h/page').mimeType(), null)
})
