'use strict'

const assert = require('node:assert/strict')
const test = require('node:test')

const { MediaType, MimeType } = require('yggdryl')

const knownMimeTypes = Object.freeze({
  OCTET_STREAM: 'application/octet-stream',
  JSON: 'application/json',
  JSON_LINES: 'application/x-ndjson',
  YAML: 'application/yaml',
  TOML: 'application/toml',
  CSV: 'text/csv',
  TSV: 'text/tab-separated-values',
  PARQUET: 'application/vnd.apache.parquet',
  ARROW_FILE: 'application/vnd.apache.arrow.file',
  ARROW_STREAM: 'application/vnd.apache.arrow.stream',
  AVRO: 'application/avro',
  ORC: 'application/vnd.apache.orc',
  PUFFIN: 'application/vnd.apache.puffin',
  PLAIN_TEXT: 'text/plain',
  ULLINK: 'text/ullink',
  FIX: 'text/fix',
  FIXUL: 'text/fixul',
  FIXML: 'text/fixml',
  MARKDOWN: 'text/markdown',
  HTML: 'text/html',
  CSS: 'text/css',
  JAVASCRIPT: 'text/javascript',
  XML: 'application/xml',
  PDF: 'application/pdf',
  CBOR: 'application/cbor',
  MESSAGE_PACK: 'application/vnd.msgpack',
  PROTOBUF: 'application/protobuf',
  SQLITE3: 'application/vnd.sqlite3',
  PNG: 'image/png',
  JPEG: 'image/jpeg',
  GIF: 'image/gif',
  WEBP: 'image/webp',
  SVG: 'image/svg+xml',
  MP3: 'audio/mpeg',
  WAV: 'audio/wav',
  OGG: 'audio/ogg',
  FLAC: 'audio/flac',
  MP4: 'video/mp4',
  WEBM: 'video/webm',
  WOFF: 'font/woff',
  WOFF2: 'font/woff2',
  TTF: 'font/ttf',
  OTF: 'font/otf',
  XLS: 'application/vnd.ms-excel',
  XLSX: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
  ODS: 'application/vnd.oasis.opendocument.spreadsheet',
  DOC: 'application/msword',
  DOCX: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
  GZIP: 'application/gzip',
  ZSTD: 'application/zstd',
  BROTLI: 'application/x-brotli',
  ZLIB: 'application/zlib',
  COMPRESS: 'application/x-compress',
  BZIP2: 'application/x-bzip2',
  XZ: 'application/x-xz',
  LZ4: 'application/x-lz4',
  SNAPPY: 'application/x-snappy-framed',
  ZIP: 'application/zip',
  SEVEN_ZIP: 'application/x-7z-compressed',
  RAR: 'application/vnd.rar',
  TAR: 'application/x-tar',
})

test('MimeType exposes the complete immutable known vocabulary and default', () => {
  assert.equal(Object.keys(knownMimeTypes).length, 60)
  assert.ok(new MimeType().equals(MimeType.OCTET_STREAM))
  const values = []
  for (const [name, canonical] of Object.entries(knownMimeTypes)) {
    const value = MimeType[name]
    assert.ok(value instanceof MimeType)
    assert.equal(value.toString(), canonical)
    assert.equal(value.isKnown(), true)
    assert.ok(MimeType.fromString(canonical).equals(value))
    assert.equal(Object.isFrozen(value), true)
    values.push(value.toString())
  }
  assert.equal(new Set(values).size, values.length)
  assert.equal('_known' in MimeType, false)
  assert.equal('_fromParts' in MediaType, false)
  assert.equal('_fromExtensions' in MediaType, false)
  assert.equal('_setEncodings' in MediaType.prototype, false)
})

test('MimeType parsing, views, JSON, ordering, and hashes stay native-owned', () => {
  const custom = new MimeType('Application/Vnd.Example+JSON')
  assert.equal(custom.toString(), 'application/vnd.example+json')
  assert.equal(custom.topLevel, 'application')
  assert.equal(custom.subtype, 'vnd.example+json')
  assert.equal(custom.structuredSuffix, 'json')
  assert.equal(custom.extension, 'json')
  assert.equal(custom.contentCoding, null)
  assert.equal(custom.format, 'json')
  assert.equal(custom.isStructured(), true)

  assert.ok(MimeType.fromExtension('.json').equals(MimeType.JSON))
  assert.ok(MimeType.fromExtension('.puffin').equals(MimeType.PUFFIN))
  assert.equal(MimeType.PUFFIN.extension, 'puffin')
  assert.equal(MimeType.PUFFIN.isBinary(), true)
  assert.equal(MimeType.PUFFIN.isStructured(), true)
  assert.equal(MimeType.PUFFIN.isTabular(), false)
  assert.ok(MimeType.fromPath('events.csv').equals(MimeType.CSV))
  assert.ok(
    MimeType.fromContentType('Application/JSON; charset="utf-8"').equals(
      MimeType.JSON,
    ),
  )
  assert.ok(MimeType.fromContentCoding('gzip').equals(MimeType.GZIP))
  assert.equal(MimeType.GZIP.contentCoding, 'gzip')
  assert.equal(MimeType.JSON.format, 'json')
  assert.ok(MimeType.fromJSON(MimeType.JSON.toJSON()).equals(MimeType.JSON))
  assert.equal(MimeType.JSON.compare(MimeType.JSON.clone()), 0)
  assert.equal(MimeType.JSON.stableHash(), MimeType.from(MimeType.JSON).stableHash())

  assert.throws(() => MimeType.fromContentType('application/json; charset'))
  assert.throws(() => MimeType.fromContentCoding('identity'))
  assert.throws(() => MimeType.from({}))
})

test('MIME and media I/O identity comes from the unencoded base', () => {
  assert.equal(MimeType.CSV.isIo(), true)
  assert.equal(
    MediaType.fromParts(MimeType.CSV, [MimeType.GZIP]).isIo(),
    true,
  )
  const directory = MimeType.fromString('inode/directory')
  assert.equal(directory.isIo(), false)
  assert.equal(new MediaType(directory).isIo(), false)
})

test('MediaType consumes iterables once and exposes detached snapshots', () => {
  let visits = 0
  function* encodings() {
    visits += 1
    yield MimeType.GZIP
    yield 'zstd'
  }
  const media = MediaType.fromParts(MimeType.CSV, encodings())
  assert.equal(visits, 1)
  assert.equal(media.toString(), 'text/csv;encodings=application/gzip,application/zstd')
  assert.ok(media.base.equals(MimeType.CSV))
  assert.deepEqual(
    media.encodings.map((value) => value.toString()),
    ['application/gzip', 'application/zstd'],
  )
  assert.ok(media.encoding.equals(MimeType.ZSTD))
  assert.equal(media.length, 2)
  assert.ok(media.at(-1).equals(MimeType.ZSTD))
  assert.equal(media.contains(MimeType.GZIP), true)
  assert.deepEqual(media.extensions, ['csv', 'gz', 'zst'])

  const detached = media.encodings
  const iterator = media[Symbol.iterator]()
  media.clearEncodings()
  assert.deepEqual(
    detached.map((value) => value.toString()),
    ['application/gzip', 'application/zstd'],
  )
  assert.deepEqual(
    [...iterator].map((value) => value.toString()),
    ['application/gzip', 'application/zstd'],
  )
  assert.deepEqual(media.encodings, [])

  const headers = MediaType.fromContentHeaders(
    'Application/JSON; Charset=utf-8',
    ' gzip ,\tbr, compress ',
  )
  assert.deepEqual(
    [...headers].map((value) => value.toString()),
    ['application/gzip', 'application/x-brotli', 'application/x-compress'],
  )
  assert.ok(new MediaType().base.equals(MimeType.OCTET_STREAM))
  assert.ok(MediaType.fromPath('events.json.gz').encoding.equals(MimeType.GZIP))
  assert.ok(MediaType.fromFileName('events.csv.zst').base.equals(MimeType.CSV))
  assert.ok(MediaType.fromExtension('json').base.equals(MimeType.JSON))
  assert.ok(
    MediaType.fromExtensions(new Set(['json', 'gz'])).equals(
      MediaType.fromParts(MimeType.JSON, [MimeType.GZIP]),
    ),
  )
  const relative = MediaType.fromString('folder/orders.csv.gz')
  assert.ok(relative.base.equals(MimeType.CSV))
  assert.deepEqual(relative.encodings, [MimeType.GZIP])
  assert.ok(
    MediaType.fromString('application/vnd.example.report+json').base.equals(
      new MimeType('application/vnd.example.report+json'),
    ),
  )
  assert.ok(
    MediaType.fromExtension(' .TBZ2\t').equals(
      MediaType.fromParts(MimeType.TAR, [MimeType.BZIP2]),
    ),
  )
})

test('MediaType mutable updates are atomic and round-trip', () => {
  const media = MediaType.fromParts(MimeType.JSON, [MimeType.GZIP])
  const before = media.toString()
  function* invalidValues() {
    yield MimeType.ZSTD
    yield {}
  }
  assert.throws(() => media.setEncodings(invalidValues()))
  assert.equal(media.toString(), before)
  assert.throws(() => media.pushEncoding(MimeType.ZIP))
  assert.equal(media.toString(), before)
  assert.throws(() => media.setBase({}))
  assert.equal(media.toString(), before)

  media.setBase(MimeType.CSV)
  media.setEncodings(new Set([MimeType.GZIP, MimeType.ZSTD]))
  media.pushEncoding(MimeType.BROTLI)
  assert.ok(media.base.equals(MimeType.CSV))
  assert.deepEqual(
    [...media].map((value) => value.toString()),
    ['application/gzip', 'application/zstd', 'application/x-brotli'],
  )
  assert.ok(MediaType.fromJSON(media.toJSON()).equals(media))
  assert.equal(media.compare(media.clone()), 0)
  assert.equal(media.stableHash(), MediaType.fromString(media.toString()).stableHash())

  assert.throws(() => MediaType.fromParts(MimeType.JSON, 'gzip'), /iterable/)
  assert.throws(() => media.setEncodings('gzip'), /iterable/)
  assert.throws(() => MediaType.fromContentHeaders(undefined, 'identity'))
})
