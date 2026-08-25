'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')

const { MediaType, MimeType, Uri, Url, Urn } = require('yggdryl')

test('URI values expose canonical components and path collections', () => {
  const uri = Uri.fromString(
    'https://example.com/archive/data.tar.gz?download=true#results',
  )
  const clone = Uri.from(uri)

  assert.equal(uri.scheme, 'https')
  assert.equal(uri.authority, 'example.com')
  assert.equal(uri.path, '/archive/data.tar.gz')
  assert.equal(uri.query, 'download=true')
  assert.equal(uri.fragment, 'results')
  assert.equal(uri.fileName, 'data.tar.gz')
  assert.equal(uri.stem, 'data.tar')
  assert.equal(uri.extension, 'gz')
  assert.deepEqual(uri.extensions, ['tar', 'gz'])
  assert.deepEqual(uri.pathSegments, ['archive', 'data.tar.gz'])
  assert.deepEqual([...uri], uri.pathSegments)
  assert.equal(uri.length, uri.pathSegments.length)
  assert.equal(uri.at(0), 'archive')
  assert.equal(uri.at(-1), 'data.tar.gz')

  assert.ok(uri.equals(clone))
  assert.equal(uri.compare(clone), 0)
  assert.equal(uri.stableHash(), clone.stableHash())
  assert.equal(typeof uri.stableHash(), 'bigint')
  assert.ok(Uri.fromString(uri.toString()).equals(uri))
  assert.ok(Uri.fromJSON(JSON.parse(JSON.stringify(uri))).equals(uri))
})

test('generic URI path joining is variadic, normalized, and immutable', () => {
  const base = Uri.fromString('https://example.com/warehouse/db?q=1#rows')

  assert.equal(
    base.joinPath('table', 'data.parquet').toString(),
    'https://example.com/warehouse/db/table/data.parquet?q=1#rows',
  )
  assert.equal(
    base.joinPath(['table', '..'], './other').toString(),
    'https://example.com/warehouse/db/other?q=1#rows',
  )
  assert.equal(
    base.joinPath('/root').toString(),
    'https://example.com/root?q=1#rows',
  )
  assert.ok(base.joinPath().equals(base))
  assert.equal(base.toString(), 'https://example.com/warehouse/db?q=1#rows')
  assert.throws(() => base.joinPath(7), TypeError)
})

test('credentials and S3 locations are parsed by the native core', () => {
  const credentials = Uri.fromString(
    'https://user:pass:word@[2001:db8::1]:8443/archive/data.parquet',
  )
  assert.equal(credentials.user, 'user')
  assert.equal(credentials.password, 'pass:word')
  assert.equal(credentials.hostname, '2001:db8::1')
  assert.equal(credentials.bucket, null)
  assert.equal(credentials.region, null)

  const bucket = Uri.fromString('s3://market-data/year=2026/data.parquet')
  assert.equal(bucket.hostname, null)
  assert.equal(bucket.bucket, 'market-data')
  assert.equal(bucket.region, null)

  const endpoint = Url.fromString(
    's3://market-data.s3.dualstack.eu-west-3.amazonaws.com/data.parquet',
  )
  assert.equal(
    endpoint.hostname,
    'market-data.s3.dualstack.eu-west-3.amazonaws.com',
  )
  assert.equal(endpoint.bucket, 'market-data')
  assert.equal(endpoint.region, 'eu-west-3')

  const compatible = Uri.fromString(
    's3://objects.example.io/archive/data.parquet',
  )
  assert.equal(compatible.hostname, 'objects.example.io')
  assert.equal(compatible.bucket, 'archive')
  assert.equal(compatible.region, null)
})

test('fromPath normalizes Windows drives and UNC shares as file URIs', () => {
  const drive = Uri.fromPath('C:\\Users\\alice\\ticks\\prices.parquet')
  assert.equal(drive.toString(), 'file:///C:/Users/alice/ticks/prices.parquet')
  assert.equal(drive.scheme, 'file')
  assert.equal(drive.authority, '')
  assert.equal(drive.path, '/C:/Users/alice/ticks/prices.parquet')
  assert.equal(drive.fileName, 'prices.parquet')
  assert.equal(drive.extension, 'parquet')
  assert.equal(drive.intoPath(), 'C:/Users/alice/ticks/prices.parquet')
  assert.equal(drive.toPath, undefined)

  const unc = Uri.fromPath('\\\\server\\share\\ticks\\prices.tar.zst')
  assert.equal(unc.toString(), 'file://server/share/ticks/prices.tar.zst')
  assert.equal(unc.scheme, 'file')
  assert.equal(unc.authority, 'server')
  assert.equal(unc.path, '/share/ticks/prices.tar.zst')
  assert.deepEqual(unc.extensions, ['tar', 'zst'])
  assert.equal(unc.intoPath(), '//server/share/ticks/prices.tar.zst')

  const unicodeUnc = Uri.fromString(
    'file://caf%C3%A9/share/market%20data%25.csv',
  )
  assert.equal(unicodeUnc.intoPath(), '//café/share/market data%.csv')
  assert.ok(Uri.fromPath(unicodeUnc.intoPath()).equals(unicodeUnc))

  assert.equal(Uri.fromPath('C:\\').toString(), 'file:///C:/')
  assert.throws(() => Uri.fromString('https://example.com/data').intoPath())
  assert.equal(Uri.fromString('file:///tmp/A%20B.csv').intoPath(), '/tmp/A B.csv')
  assert.equal(Uri.fromString('file://server').intoPath(), '//server/')
  assert.throws(() => Uri.fromString('file:///tmp/a%2Fb.csv').intoPath())
  assert.throws(() => Uri.fromString('file:///tmp/a%5Cb.csv').intoPath())
  assert.throws(() => Uri.fromString('file:///tmp/data.csv?download=1').intoPath())
  assert.throws(() => Uri.fromString('file:///tmp/data.csv#row-1').intoPath())
  assert.throws(() => Uri.fromString('file:').intoPath())
  assert.throws(() => Uri.fromString('file:///tmp/%FF.csv').intoPath())
  assert.throws(() => Uri.fromString('file://user%40host/share/data.csv').intoPath())
  assert.throws(() => Uri.fromString('file:///%43%3A/data.csv').intoPath())
  assert.throws(() => Uri.fromString('file:///tmp/%C2%85/data.csv').intoPath())
})

test('URL conversion is validated by the native URI core', () => {
  const uri = Uri.fromString('https://example.com/b/report.csv?q=1#top')
  const url = Url.fromUri(uri)
  const clone = Url.from(url)

  assert.equal(url.scheme, 'https')
  assert.equal(url.authority, 'example.com')
  assert.equal(url.path, '/b/report.csv')
  assert.equal(url.query, 'q=1')
  assert.equal(url.fragment, 'top')
  assert.equal(url.fileName, 'report.csv')
  assert.deepEqual([...url], url.pathSegments)
  assert.ok(url.equals(clone))
  assert.ok(Uri.from(url).equals(uri))
  assert.ok(new Uri(url).equals(uri))
  assert.ok(url.intoUri().equals(Uri.fromString(url.toString())))
  assert.ok(uri.intoUrl().equals(url))
  assert.equal(url.toUri, undefined)
  assert.equal(uri.toUrl, undefined)
  assert.ok(Url.fromString(url.toString()).equals(url))
  assert.ok(Url.fromJSON(JSON.parse(JSON.stringify(url))).equals(url))
  assert.ok(
    Url.fromPath('C:\\data\\report.csv').equals(
      Url.fromString('file:///C:/data/report.csv'),
    ),
  )
  assert.equal(Url.fromPath('C:\\data\\report.csv').intoPath(), 'C:/data/report.csv')

  assert.throws(() => Url.fromString('urn:isbn:9780131103627'))
  assert.throws(() => Url.fromUri(Uri.fromString('mailto:user@example.com')))
  assert.throws(() => Url.from(Urn.fromString('urn:isbn:9780131103627')), /URL/)
})

test('URN values expose namespace and namespace-specific string', () => {
  const uri = Uri.fromString('URN:ISBN:9780131103627')
  const urn = Urn.fromUri(uri)
  const clone = Urn.from(urn)

  assert.equal(urn.scheme, 'urn')
  assert.equal(urn.authority, '')
  assert.equal(urn.path, 'isbn:9780131103627')
  assert.equal(urn.query, null)
  assert.equal(urn.fragment, null)
  assert.deepEqual([...urn], urn.pathSegments)
  assert.equal(urn.namespace, 'isbn')
  assert.equal(urn.namespaceSpecific, '9780131103627')
  assert.ok(urn.equals(clone))
  assert.ok(Uri.from(urn).equals(urn.intoUri()))
  assert.ok(new Uri(urn).equals(urn.intoUri()))
  assert.equal(urn.compare(clone), 0)
  assert.equal(typeof urn.stableHash(), 'bigint')
  assert.ok(urn.intoUri().equals(Uri.fromString(urn.toString())))
  assert.ok(uri.intoUrn().equals(urn))
  assert.ok(Urn.fromString(urn.toString()).equals(urn))
  assert.ok(Urn.fromJSON(JSON.parse(JSON.stringify(urn))).equals(urn))

  assert.throws(() => Urn.fromString('https://example.com/resource'))
  assert.throws(() => Urn.fromString('urn::missing-namespace'))
  assert.throws(() => Urn.from(Url.fromString('https://example.com')), /URN/)
  assert.throws(() => uri.intoUrl())
})

test('scheme-less input parses as a file URI instead of failing', () => {
  const absolute = Uri.fromString('/relative/path')
  assert.equal(absolute.scheme, 'file')
  assert.equal(absolute.authority, '')
  assert.equal(absolute.path, '/relative/path')
  assert.equal(absolute.toString(), 'file:///relative/path')

  const relative = Uri.fromString('noscheme.example/path')
  assert.equal(relative.scheme, 'file')
  assert.equal(relative.toString(), 'file:noscheme.example/path')
  assert.deepEqual(relative.pathSegments, ['noscheme.example', 'path'])
})

test('URI parsing rejects malformed schemes, authorities, and escapes', () => {
  assert.throws(() => Uri.fromString('://missing.example'))
  assert.throws(() => Uri.fromString('1http://example.com'))
  assert.throws(() => Uri.fromString('https://[invalid/path'))
  assert.throws(() => Uri.fromString('https://foo[bar]/path'))
  assert.throws(() => Uri.fromString('https://example.com:notaport/path'))
  assert.throws(() => Uri.fromString('https://example.com/%GG'))
  assert.throws(() => Uri.fromString('https://example.com/space here'))
})

test('filename mutations stay native, atomic, and preserve URI components', () => {
  const uri = Uri.fromString(
    'https://example.com/archive/report.tar.gz?q=1#part',
  )
  const clone = uri.clone()

  uri.setStem('renamed')
  assert.equal(uri.toString(), 'https://example.com/archive/renamed.gz?q=1#part')
  uri.setExtension('zst')
  uri.setExtensions(['csv', 'gz'])
  assert.equal(
    uri.toString(),
    'https://example.com/archive/renamed.csv.gz?q=1#part',
  )
  assert.equal(uri.removeExtension(), true)
  assert.deepEqual(uri.extensions, ['csv'])
  assert.equal(uri.clearExtensions(), true)
  assert.equal(uri.extension, null)
  assert.equal(
    clone.toString(),
    'https://example.com/archive/report.tar.gz?q=1#part',
  )

  const unchanged = uri.toString()
  for (const invalid of ['', 'bad/name', 'bad?name', 'bad#name', 'bad%']) {
    assert.throws(() => uri.setFileName(invalid))
    assert.equal(uri.toString(), unchanged)
  }
  assert.throws(() => uri.setExtensions(['json', 'bad/name']))
  assert.equal(uri.toString(), unchanged)

  const authorityOnly = Url.fromString('https://example.com?q=1#part')
  authorityOnly.setFileName('data.json')
  assert.equal(authorityOnly.toString(), 'https://example.com/data.json?q=1#part')

  const urn = Urn.fromString('urn:example:reports/data.csv?=raw#rows')
  urn.setFileName('renamed.json')
  assert.equal(urn.toString(), 'urn:example:reports/renamed.json?=raw#rows')
  assert.equal(urn.namespace, 'example')
  assert.equal(urn.fileName, 'renamed.json')
  assert.equal(urn.stem, 'renamed')
})

test('URI MIME and media inference uses native compound suffix tables', () => {
  const uri = Uri.fromString(
    'https://example.com/report.csv.gz.zst?q=1#part',
  )

  assert.ok(uri.mimeType.equals(MimeType.fromString('application/zstd')))
  assert.ok(uri.mediaType.base.equals(MimeType.fromString('text/csv')))
  assert.deepEqual(
    uri.mediaType.encodings.map((value) => value.toString()),
    ['application/gzip', 'application/zstd'],
  )

  uri.setMimeType('application/json')
  assert.equal(
    uri.toString(),
    'https://example.com/report.csv.gz.json?q=1#part',
  )

  const media = MediaType.fromParts('text/csv', [
    'application/gzip',
    'application/zstd',
  ])
  uri.setMediaType(media)
  assert.equal(
    uri.toString(),
    'https://example.com/report.csv.gz.zst?q=1#part',
  )

  const unchanged = uri.toString()
  assert.throws(
    () => uri.setMimeType('application/vnd.example'),
    /preferred filename extension/,
  )
  assert.equal(uri.toString(), unchanged)
  assert.throws(
    () =>
      uri.setMediaType(
        MediaType.fromParts('application/vnd.example', ['application/gzip']),
      ),
    /preferred filename extension/,
  )
  assert.equal(uri.toString(), unchanged)

  const urn = Urn.fromString('urn:example:reports/data.json')
  urn.setMediaType('text/csv;encodings=application/gzip')
  assert.equal(urn.toString(), 'urn:example:reports/data.csv.gz')
  assert.equal(urn.namespace, 'example')
})

test('a URL answers the naming questions a path answers', () => {
  const url = Url.fromString('file:///lake/trades/part-0.tar.gz')

  assert.equal(url.name, 'part-0.tar.gz')
  assert.equal(url.stem, 'part-0.tar')
  assert.equal(url.suffix, '.gz')
  assert.deepEqual(url.suffixes, ['.tar', '.gz'])
  assert.deepEqual(url.parts, ['lake', 'trades', 'part-0.tar.gz'])
  assert.deepEqual(url.parts, [...url])
  assert.ok(url.isAbsolute())
  assert.equal(url.asPosix(), '/lake/trades/part-0.tar.gz')
  assert.equal(url.asUri(), url.toString())
  assert.equal(Url.fromString('https://example.com').name, '')
  assert.equal(Url.fromString('https://example.com').suffix, '')
})

test('joining and climbing a URL preserve every other component', () => {
  const root = Url.fromString('file:///lake')

  assert.equal(root.joinpath('trades', 'part-0.arrows').toString(), 'file:///lake/trades/part-0.arrows')
  assert.equal(root.joinpath('trades').joinpath('part-0.arrows').toString(), 'file:///lake/trades/part-0.arrows')
  assert.equal(root.joinpath().toString(), root.toString())

  const query = Url.fromString('https://example.com/a/b/c?q=1#frag')
  assert.equal(query.joinpath('d').toString(), 'https://example.com/a/b/c/d?q=1#frag')
  assert.equal(query.parent.path, '/a/b')
  assert.deepEqual(query.parents.map((parent) => parent.path), ['/a/b', '/a', '/'])
  // A location at the root is its own parent.
  assert.equal(Url.fromString('file:///').parent.toString(), 'file:///')
  assert.throws(() => root.joinpath(7), TypeError)
})

test('renaming a URL rewrites only the final component', () => {
  const url = Url.fromString('file:///lake/part-0.arrows?q=1')

  assert.equal(url.withName('part-1.arrows').name, 'part-1.arrows')
  assert.equal(url.withStem('part-1').name, 'part-1.arrows')
  assert.equal(url.withSuffix('.parquet').name, 'part-0.parquet')
  assert.equal(url.withSuffix('parquet').name, 'part-0.parquet')
  assert.equal(url.withSuffix('').name, 'part-0')
  assert.equal(url.withName('part-1.arrows').query, 'q=1')
  // A persistent update leaves the source value alone.
  assert.equal(url.name, 'part-0.arrows')
  assert.throws(() => url.withName('bad/name'))
})

test('matching a URL follows the gitignore rule', () => {
  const url = Url.fromString('file:///lake/year=2024/part-0.parquet')

  assert.ok(url.match('*.parquet'))
  assert.ok(url.match('lake/**/part-?.parquet'))
  assert.ok(!url.match('lake/*.parquet'))
  assert.ok(url.fullMatch('lake/**/part-0.parquet'))
  assert.ok(Url.fromString('file:///lake/**/*.parquet').isGlob())
  assert.ok(!url.isGlob())
})

test('a URL says what it is relative to', () => {
  const root = Url.fromString('file:///lake')
  const url = Url.fromString('file:///lake/year=2024/part-0.parquet')

  assert.equal(url.relativeTo(root), 'year=2024/part-0.parquet')
  assert.equal(url.relativeTo('file:///lake'), 'year=2024/part-0.parquet')
  assert.ok(url.isRelativeTo(root))
  assert.ok(!url.isRelativeTo(Url.fromString('file:///other')))
  assert.throws(() => url.relativeTo('file:///other'), /not in the subpath/)
})

test('the file system predicates answer for a local URL only', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-url-'))
  const leaf = path.join(root, '.staging')
  fs.mkdirSync(leaf)
  try {
    const url = Url.fromPath(root)

    assert.ok(url.exists())
    assert.ok(url.isDir())
    assert.ok(!url.isFile())
    assert.ok(url.joinpath('.staging').isPrivate())
    assert.ok(!url.isPrivate())

    // A remote URL answers without a round trip: it is simply not local.
    const remote = Url.fromString('https://example.test/ticks.csv')
    assert.ok(!remote.exists())
    assert.ok(!remote.isDir())
    assert.ok(!remote.isFile())
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
})

test('Hive partitions are read off the URL path', () => {
  const url = Url.fromString('file:///lake/year=2024/month=01/part-0.parquet')

  assert.deepEqual(url.partitions, [
    { column: 'year', value: '2024' },
    { column: 'month', value: '01' },
  ])
  assert.equal(url.partition('month'), '01')
  assert.equal(url.partition('day'), null)
  assert.deepEqual(Url.fromString('file:///lake/part-0.parquet').partitions, [])
})
