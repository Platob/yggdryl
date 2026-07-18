'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { MimeType, MimeCatalog } = yggdryl.mimetype

// PNG magic — the eight-byte signature every PNG file starts with.
const PNG_MAGIC = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])

test('the mimetype namespace exposes MimeType and MimeCatalog', () => {
  assert.equal(typeof MimeType, 'function')
  assert.equal(typeof MimeCatalog, 'function')
})

test('parse drops parameters and splits type/subtype', () => {
  const json = MimeType.parse('Application/JSON; charset=utf-8')
  assert.equal(json.essence, 'application/json') // lowercased, parameters dropped
  assert.equal(json.type, 'application')
  assert.equal(json.subtype, 'json')
  assert.equal(json.toString(), 'application/json')

  // A non-essence input is a guided error.
  assert.throws(() => MimeType.parse('notamime'), /type\/subtype/)
})

test('default-catalog resolution: fromExtension / fromName / fromMagic / guess', () => {
  assert.equal(MimeType.fromExtension('png').essence, 'image/png')
  assert.equal(MimeType.fromName('report.pdf').essence, 'application/pdf')
  assert.equal(MimeType.fromMagic(PNG_MAGIC).essence, 'image/png')

  // Unknown resolutions are null (a justified absence).
  assert.equal(MimeType.fromExtension('zzz'), null)
  assert.equal(MimeType.fromName('noext'), null)
  assert.equal(MimeType.fromMagic(Buffer.from('plain text')), null)

  // guess always has an answer: magic wins, then the name, else octet-stream.
  assert.equal(MimeType.guess('x.txt', PNG_MAGIC).essence, 'image/png') // magic beats name
  assert.equal(MimeType.guess('x.pdf', Buffer.alloc(0)).essence, 'application/pdf') // then name
  assert.equal(MimeType.guess('mystery', Buffer.alloc(0)).essence, 'application/octet-stream')
})

test('octetStream is the fallback', () => {
  const octet = MimeType.octetStream()
  assert.equal(octet.essence, 'application/octet-stream')
  assert.ok(octet.isOctetStream())
  assert.ok(!MimeType.parse('text/plain').isOctetStream())
})

test('constructor carries extensions + magic; hasExtension / matchesMagic / getters', () => {
  const foo = new MimeType('Application/X-Foo', ['.Foo', 'foo2'], [Buffer.from([1, 2, 3])])
  assert.equal(foo.essence, 'application/x-foo') // essence lowercased
  assert.deepEqual(foo.extensions, ['foo', 'foo2']) // lowercased, leading dot stripped
  assert.deepEqual(foo.magic.map((b) => [...b]), [[1, 2, 3]])

  assert.ok(foo.hasExtension('FOO')) // case-insensitive
  assert.ok(foo.hasExtension('.foo2')) // leading dot ignored
  assert.ok(!foo.hasExtension('bar'))
  assert.ok(foo.matchesMagic(Buffer.from([1, 2, 3, 9]))) // prefix match
  assert.ok(!foo.matchesMagic(Buffer.from([9, 9])))

  // Omitted extensions/magic default to empty.
  const bare = new MimeType('text/plain')
  assert.deepEqual(bare.extensions, [])
  assert.deepEqual(bare.magic, [])
})

test('copy is an independent value; essence identity holds', () => {
  const png = MimeType.fromExtension('png')
  const dup = png.copy()
  assert.ok(dup.equals(png))
  assert.equal(dup.hashCode(), png.hashCode())
  assert.ok(!png.equals(MimeType.parse('image/jpeg')))
})

test('serializeBytes / deserializeBytes round-trip the essence bytes', () => {
  const json = MimeType.parse('application/json')
  const raw = json.serializeBytes()
  assert.ok(Buffer.isBuffer(raw))
  assert.equal(raw.toString('utf8'), 'application/json')
  const back = MimeType.deserializeBytes(raw)
  assert.ok(back.equals(json))
  assert.equal(back.hashCode(), json.hashCode())

  // The byte form is essence-only — a catalog entry's extensions/magic are not carried.
  assert.deepEqual(MimeType.deserializeBytes(MimeType.fromExtension('png').serializeBytes()).extensions, [])

  // Non-UTF-8 / bad essence bytes are a guided error.
  assert.throws(() => MimeType.deserializeBytes(Buffer.from([0xff, 0xfe])), /UTF-8/)
  assert.throws(() => MimeType.deserializeBytes(Buffer.from('notamime')), /type\/subtype/)
})

test('value semantics: equal essences hash equal', () => {
  const a = MimeType.parse('text/html')
  const b = MimeType.parse('Text/HTML; q=0.9') // parameters + case ignored
  assert.ok(a.equals(b))
  assert.equal(a.hashCode(), b.hashCode())
})

// -------------------------------------------------------------------------------------
// MimeCatalog
// -------------------------------------------------------------------------------------

test('MimeCatalog.defaults seeds the built-in known types', () => {
  const catalog = MimeCatalog.defaults()
  assert.ok(catalog.len() > 0)
  assert.ok(!catalog.isEmpty())
  assert.equal(catalog.fromName('a.json').essence, 'application/json')
  assert.equal(catalog.fromExtension('png').essence, 'image/png')
  assert.equal(catalog.fromMime('image/jpeg').essence, 'image/jpeg')
  assert.equal(catalog.fromMagic(PNG_MAGIC).essence, 'image/png')
  assert.equal(catalog.fromExtension('zzz'), null)

  // types() lists the registered entries.
  assert.ok(catalog.types().every((t) => t instanceof MimeType))
  assert.equal(catalog.types().length, catalog.len())
})

test('a fresh catalog is empty and grows by register / with', () => {
  const empty = new MimeCatalog()
  assert.equal(empty.len(), 0)
  assert.ok(empty.isEmpty())
  assert.equal(empty.fromExtension('foo'), null)

  const foo = new MimeType('application/x-foo', ['foo'], [])
  empty.register(foo)
  assert.equal(empty.len(), 1)
  assert.equal(empty.fromExtension('foo').essence, 'application/x-foo')

  // A later registration with the same essence overrides the earlier one.
  empty.register(new MimeType('application/x-foo', ['foo', 'f'], []))
  assert.equal(empty.len(), 1)
  assert.equal(empty.fromExtension('f').essence, 'application/x-foo')

  // `with` is the chainable, non-mutating builder.
  const built = new MimeCatalog().with(foo).with(new MimeType('text/x-bar', ['bar'], []))
  assert.equal(built.len(), 2)
  assert.equal(new MimeCatalog().with(foo).fromExtension('foo').essence, 'application/x-foo')

  assert.match(new MimeCatalog().toString(), /MimeCatalog\(<0 types>\)/)
})

test('MimeCatalog.copy is an independent clone', () => {
  const base = new MimeCatalog().with(new MimeType('application/x-foo', ['foo'], []))
  const dup = base.copy()
  dup.register(new MimeType('text/x-bar', ['bar'], []))
  assert.equal(base.len(), 1) // original untouched
  assert.equal(dup.len(), 2)
})

// -------------------------------------------------------------------------------------
// names / extension / isCompression / fromAlias (the enriched MimeType surface)
// -------------------------------------------------------------------------------------

test('extension is the primary (first) extension, or null when there is none', () => {
  assert.equal(MimeType.fromExtension('jpeg').extension, 'jpg') // primary of [jpg, jpeg]
  assert.equal(MimeType.fromExtension('gz').extension, 'gz')
  assert.equal(MimeType.parse('application/json').extension, null) // parsed -> no extensions
})

test('names lists the short aliases; the constructor takes an optional names[]', () => {
  // Catalog entries carry their aliases (application/gzip is known as gzip / gz).
  assert.deepEqual(MimeType.fromExtension('gz').names, ['gzip', 'gz'])
  assert.deepEqual(MimeType.parse('application/json').names, []) // parsed -> no names

  // The trailing `names?` constructor arg maps to the core `named` builder (lowercased).
  const foo = new MimeType('application/x-foo', ['foo'], [], ['F-Oo', 'foo-alias'])
  assert.deepEqual(foo.names, ['f-oo', 'foo-alias'])
  assert.deepEqual(foo.extensions, ['foo'])
  // Omitted names default to empty; the earlier (essence, extensions, magic) calls still work.
  assert.deepEqual(new MimeType('application/x-bar', ['bar'], []).names, [])

  // Names/extensions are catalog metadata dropped by the byte codec: serializeBytes is
  // essence-only, so the round-tripped value is a bare essence.
  assert.equal(foo.serializeBytes().toString('utf8'), 'application/x-foo')
  const back = MimeType.deserializeBytes(foo.serializeBytes())
  assert.deepEqual(back.names, []) // names not carried across the byte form
  assert.deepEqual(back.extensions, [])
  const parsed = MimeType.parse('application/x-foo') // also essence-only
  assert.ok(back.equals(parsed)) // two essence-only values with the same essence are equal
  assert.equal(back.hashCode(), parsed.hashCode())
})

test('isCompression flags the compression formats', () => {
  assert.ok(MimeType.fromExtension('gz').isCompression())
  assert.ok(MimeType.fromExtension('zst').isCompression())
  assert.ok(MimeType.parse('application/x-xz').isCompression())
  assert.ok(MimeType.parse('application/zlib').isCompression())
  assert.ok(!MimeType.fromExtension('json').isCompression())
  assert.ok(!MimeType.parse('image/png').isCompression())
})

test('fromAlias resolves a short name via the default catalog, else null', () => {
  assert.equal(MimeType.fromAlias('gzip').essence, 'application/gzip')
  assert.equal(MimeType.fromAlias('zstd').essence, 'application/zstd')
  assert.equal(MimeType.fromAlias('json').essence, 'application/json')
  assert.equal(MimeType.fromAlias('nope'), null) // unknown alias -> justified null
})
