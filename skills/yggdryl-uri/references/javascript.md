# yggdryl-uri in JavaScript

`const { Uri, Url, Urn, Arn } = require('yggdryl')`. Components are getters
in camelCase; there is no class inheritance, so `Uri.from(...)` answers a
`Uri` and `intoUrl()` / `intoUrn()` / `intoArn()` narrow it. Errors are thrown
`Error`s carrying the native message and byte offset.

## Parse any identifier and read its components

`Uri.from(value)` / `new Uri(value)` accept text or another identifier.
`query` and `fragment` are getters answering the **stored** form (JavaScript
has no decoding view).

```javascript
const assert = require('node:assert/strict')
const { Uri } = require('yggdryl')

const uri = Uri.from('HTTPS://example.test/archive/caf%c3%a9.tar.gz?as%20of=1#part')

assert.equal(uri.toString(), 'https://example.test/archive/caf%C3%A9.tar.gz?as%20of=1#part')
assert.equal(uri.scheme, 'https')
assert.equal(uri.authority, 'example.test')
assert.equal(uri.path, '/archive/caf%C3%A9.tar.gz')
assert.equal(uri.query, 'as%20of=1')
assert.equal(uri.fragment, 'part')
assert.equal(decodeURIComponent(uri.pathSegments.at(-1)), 'café.tar.gz')
```

## Compare two spellings of one resource

Parsing canonicalizes, so `equals`, `compare` and `stableHash` compare
resources, not text. A string with no scheme token is a file path.

```javascript
const assert = require('node:assert/strict')
const { Uri } = require('yggdryl')

assert.equal(Uri.from('file:///C:\\Users\\Ada\\report.parquet').toString(), 'file:///C:/Users/Ada/report.parquet')
assert.equal(Uri.from('/var/lib/data.arrow').toString(), 'file:///var/lib/data.arrow')
assert.equal(Uri.from('data/ticks.csv').toString(), 'file:data/ticks.csv')
assert.equal(Uri.from('/data/2026-08-16T00:00:00/p.parquet').scheme, 'file')
assert.throws(() => Uri.from('bad scheme://x'))

const a = Uri.from('HTTPS://x.test/caf%c3%a9')
const b = Uri.from('https://x.test/caf%C3%A9')
assert.ok(a.equals(b))
assert.equal(a.compare(b), 0)
assert.equal(a.stableHash(), b.stableHash())
assert.equal(typeof a.stableHash(), 'bigint')
assert.ok(Uri.fromJSON(a.toJSON()).equals(a))
```

## Narrow to a location, a name or an ARN

`intoUrl()`/`intoUrn()`/`intoArn()` and `Url.fromUri` refuse what a value is
not. `Url.fromString` is strict; `Url.from` / `new Url` is the location door:
it roots a relative path at the working directory and resolves a name.

```javascript
const assert = require('node:assert/strict')
const { Arn, Uri, Url, Urn } = require('yggdryl')

const uri = Uri.from('https://example.test/a/data.json?raw=true')
assert.ok(Uri.from(uri.intoUrl()).equals(uri))

const urn = Uri.from('URN:ISBN:9780131103627').intoUrn()
assert.ok(urn instanceof Urn)
assert.equal(urn.toString(), 'urn:isbn:9780131103627')
assert.ok(Uri.from('arn:aws:s3:::b/k').intoArn() instanceof Arn)

assert.throws(() => urn.intoUri().intoUrl())
assert.throws(() => Url.fromString('mailto:user@example.test'))
assert.throws(() => Url.fromString('data/ticks.csv')) // strict: no relative text
assert.throws(() => Url.fromUri(urn)) // strict: a name is not a location

const rooted = Url.from('data/ticks.csv') // the location door
assert.equal(rooted.scheme, 'file')
assert.ok(rooted.toString().endsWith('/data/ticks.csv'))
assert.equal(Url.from(new Arn('arn:aws:s3:::market-data/p.parquet')).toString(), 's3://market-data/p.parquet')
```

## Convert platform paths

`Uri.fromPath` / `Url.fromPath` detect drives and UNC shares textually (the
same answer on every host) and encode names; `intoPath()` decodes back and
refuses anything that is not a `file:` path.

```javascript
const assert = require('node:assert/strict')
const { Uri, Url } = require('yggdryl')

const uri = Uri.fromPath('C:\\Users\\Ada Lovelace\\report.parquet')
assert.equal(uri.toString(), 'file:///C:/Users/Ada%20Lovelace/report.parquet')
assert.equal(uri.authority, '')
assert.equal(uri.intoPath(), 'C:/Users/Ada Lovelace/report.parquet')
assert.ok(Uri.fromPath(uri.intoPath()).equals(uri))

const unc = Uri.fromPath('\\\\server\\share\\prices\\ticks.csv')
assert.equal(unc.toString(), 'file://server/share/prices/ticks.csv')
assert.equal(unc.intoPath(), '//server/share/prices/ticks.csv')

assert.ok(Url.fromPath('relative/x.csv').toString().endsWith('/relative/x.csv'))
assert.throws(() => Url.fromString('https://example.test/data.csv').intoPath())
assert.throws(() => Uri.from('file:///lake/a%2Fb').intoPath())
```

## Read and rename the filename; read the media type

Filename getters and in-place setters; `Url` adds `name`, `suffix`,
`suffixes` and the copying `withName`/`withStem`/`withSuffix`. A refused name
changes nothing.

```javascript
const assert = require('node:assert/strict')
const { MediaType, MimeType, Uri, Url } = require('yggdryl')

const uri = Uri.from('https://example.test/archive/report.csv.gz?q=1#part')
assert.deepEqual([uri.fileName, uri.stem, uri.extension], ['report.csv.gz', 'report.csv', 'gz'])
assert.deepEqual(uri.extensions, ['csv', 'gz'])
assert.ok(uri.mimeType.equals(MimeType.from('application/gzip')))
assert.ok(uri.mediaType.base.equals(MimeType.from('text/csv')))
assert.deepEqual(uri.mediaType.encodings.map(String), ['application/gzip'])

uri.setStem('renamed')
assert.equal(uri.toString(), 'https://example.test/archive/renamed.gz?q=1#part')
uri.setMediaType(MediaType.fromParts('application/json', ['application/zstd']))
assert.equal(uri.fileName, 'renamed.json.zst')
assert.equal(uri.clearExtensions(), true)

const unchanged = uri.toString()
assert.throws(() => uri.setFileName('bad/name'))
assert.throws(() => uri.setMimeType('application/vnd.example'), /preferred filename extension/)
assert.equal(uri.toString(), unchanged)

const url = Url.from('file:///lake/trades.csv.gz')
assert.deepEqual([url.name, url.suffix, url.suffixes], ['trades.csv.gz', '.gz', ['.csv', '.gz']])
assert.equal(url.withSuffix('.zst').toString(), 'file:///lake/trades.csv.zst')
assert.equal(url.withName('quotes.csv').toString(), 'file:///lake/quotes.csv')
```

## Walk and join paths

`Url.joinpath(...others)` and `Uri.joinPath(...others)` are variadic, like
`path.join`, and resolve `.`/`..`; `parts`, `parent` and `parents` are `Url`
getters. Joins take URI text.

```javascript
const assert = require('node:assert/strict')
const { Uri, Url } = require('yggdryl')

const url = Url.from('https://example.test/a/b/c?q=1#frag')
assert.equal(url.joinpath('../d').toString(), 'https://example.test/a/b/d?q=1#frag')
assert.equal(url.joinpath('d', 'e').toString(), 'https://example.test/a/b/c/d/e?q=1#frag')
assert.equal(Uri.from('https://x.test/a').joinPath('b', 'c').toString(), 'https://x.test/a/b/c')

assert.deepEqual(url.parts, ['a', 'b', 'c'])
assert.deepEqual(url.parents.map((parent) => parent.path), ['/a/b', '/a', '/'])
assert.equal(url.parent.path, '/a/b')
assert.deepEqual([url.length, url.at(0), url.at(-1)], [3, 'a', 'c'])
assert.deepEqual([...url], url.pathSegments)

assert.equal(Url.from('file:///lake/year=2024/p.bin').relativeTo('file:///lake'), 'year=2024/p.bin')
assert.ok(Url.from('file:///lake/p.bin').isRelativeTo('file:///lake'))
assert.throws(() => Url.from('file:///lake').joinpath('100%.csv')) // URI text: `%` must be an escape
assert.equal(Url.from('file:///lake').joinpath('100%25.csv').fileName, '100%25.csv')
```

## Read credentials and object-store locations

Everything is read off the authority, with no request. A first part ending
`.com`/`.io`, with a port, an IP literal, or `localhost` is the host;
otherwise it is the bucket.

```javascript
const assert = require('node:assert/strict')
const { Uri } = require('yggdryl')

const secured = Uri.from('https://user:pass:word@example.com/data')
assert.deepEqual([secured.user, secured.password, secured.hostname], ['user', 'pass:word', 'example.com'])

const s3 = Uri.from('s3://trades.s3.eu-west-3.amazonaws.com/lake/part.parquet')
assert.deepEqual([s3.bucket, s3.region, s3.key], ['trades', 'eu-west-3', 'lake/part.parquet'])
assert.ok(s3.isVirtualHosted())

const minio = Uri.from('s3://localhost:9000/trades/lake/')
assert.deepEqual([minio.hostname, minio.bucket, minio.key], ['localhost', 'trades', 'lake/'])

const azure = Uri.from('abfss://lake@trades.dfs.core.windows.net/part.parquet')
assert.deepEqual([azure.bucket, azure.account, azure.key], ['lake', 'trades', 'part.parquet'])
assert.equal(Uri.from('gs://trades.storage.googleapis.com/x').storeEndpoint, 'storage.googleapis.com')
```

## Detect and match globs

`isGlob()` detects a `*` pattern in a URL path; `match` follows the
`.gitignore` rule (no `/`: the name at any depth; a `/`: anchored at the
root). `globParts` and `matches_glob_under` are Rust and Python only; the
listing itself is `IOBase.glob`.

```javascript
const assert = require('node:assert/strict')
const { Url } = require('yggdryl')

assert.ok(Url.from('file:///lake/trades/year=2024/**/*.parquet').isGlob())
assert.ok(!Url.from('file:///lake/trades.arrows').isGlob())

const part = Url.from('file:///lake/trades/year=2024/month=01/part-0.parquet')
assert.ok(part.match('*.parquet'))
assert.ok(part.match('lake/**/part-?.parquet'))
assert.ok(!part.match('lake/*.parquet'))
assert.ok(part.fullMatch('/lake/**/*.parquet'))
```

## Read Hive partitions and select leaves by them

`partitions` answers `{ column, value }` entries in path order and
`partition(column)` one value or `null`. A handle's `childrenWhere` selects
leaves by path with no call per file.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { IOBase, Url } = require('yggdryl')

const part = Url.from('file:///lake/year=2024/month=01/part-0.parquet')
assert.deepEqual(part.partitions, [
  { column: 'year', value: '2024' },
  { column: 'month', value: '01' },
])
assert.equal(part.partition('month'), '01')
assert.equal(part.partition('day'), null)

const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-')), 'lake')
for (const year of ['2024', '2025']) {
  new IOBase(root).joinpath(`year=${year}/month=01/part-0.bin`).writeBytes(Buffer.from('x'))
}
const selected = [...new IOBase(root).childrenWhere({ year: '2024' })]
assert.equal(selected.length, 1)
assert.deepEqual(selected[0].partitions[0], { column: 'year', value: '2024' })
fs.rmSync(root, { recursive: true, force: true })
```

## Resolve a URN to a location

A URN's namespace and its `:`-separated parts are a relative path;
`resolve(base)` places it under a store, `locator()` under the working
directory.

```javascript
const assert = require('node:assert/strict')
const { Uri, Url, Urn } = require('yggdryl')

const urn = Urn.from('urn:lake:trades:2026:part.parquet')
assert.deepEqual([urn.namespace, urn.namespaceSpecific], ['lake', 'trades:2026:part.parquet'])
assert.equal(urn.locatorPath(), 'lake/trades/2026/part.parquet')
assert.equal(
  urn.resolve('s3://market-data/warehouse/').toString(),
  's3://market-data/warehouse/lake/trades/2026/part.parquet',
)

const located = urn.locator()
assert.equal(located.scheme, 'file')
assert.equal(Uri.from('urn:lake:trades:2026:part.parquet').locator().toString(), located.toString())
assert.equal(Url.from(urn).toString(), located.toString())

assert.throws(() => Urn.from('urn:example:a::b').locatorPath())
assert.throws(() => Urn.from('urn:isbn:'))
```

## Read an ARN and locate an S3 object

An ARN is five AWS fields plus a service-owned resource. Only an Amazon S3
bucket ARN (and an S3 Tables ARN) locates a URL.

```javascript
const assert = require('node:assert/strict')
const { Arn } = require('yggdryl')

const object = Arn.from('arn:aws:s3:::market-data/2026/part.parquet')
assert.deepEqual([object.partition, object.service, object.region, object.account], ['aws', 's3', null, null])
assert.deepEqual([object.bucket, object.key, object.fileName], ['market-data', '2026/part.parquet', 'part.parquet'])
assert.equal(object.locator().toString(), 's3://market-data/2026/part.parquet')

const fn = Arn.from('arn:aws:lambda:us-east-1:123456789012:function:trades:1')
assert.deepEqual([fn.resourceType, fn.resourceId, fn.resourceSeparator], ['function', 'trades:1', ':'])
assert.throws(() => fn.locator())

const table = Arn.from('arn:aws:s3tables:us-east-1:123456789012:bucket/lake/table/t-a1')
assert.deepEqual([table.bucket, table.table, table.key], ['lake', 't-a1', null])
assert.equal(table.locator().toString(), 's3tables://lake/t-a1')
assert.equal(Arn.fromParts('aws', 's3', '', '', 'b/k').toString(), 'arn:aws:s3:::b/k')
```

## Ask the local filesystem

`exists()`, `isDir()`, `isFile()` look at the disk for a `file:` URL and
answer `false` for any other scheme, with no network call; `isPrivate()`
judges the last segment.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { Url } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
fs.writeFileSync(path.join(root, 'ticks.csv'), 'symbol\n')

const folder = Url.fromPath(root)
assert.ok(folder.isDir())
assert.ok(folder.joinpath('ticks.csv').isFile())
assert.ok(Url.from('file:///project/.git').isPrivate())
assert.equal(Url.from('https://example.test/ticks.csv').exists(), false)

fs.rmSync(root, { recursive: true, force: true })
```

## Declare a column of URLs or URNs

`fields.url(name)` and `fields.urn(name)` build the two `uri`-family fields;
text entering either is parsed and canonicalized, and relative text or the
other leaf's identifier is refused.

```javascript
const assert = require('node:assert/strict')
const { DataType, fields, json } = require('yggdryl')

const location = fields.url('location', { nullable: false })
const name = fields.urn('name', { nullable: false })
assert.equal(new DataType('urn').kind, 'text')

const located = json.loads('"HTTPS://example.com/a"', { field: location, scalar: true })
assert.equal(located.asJs(), 'https://example.com/a')
const named = json.loads('"URN:ISBN:0451450523"', { field: name, scalar: true })
assert.equal(named.asJs(), 'urn:isbn:0451450523')
assert.throws(() => json.loads('"./relative"', { field: location, scalar: true }))
```

## Not bound in JavaScript

Rust and Python only - do not invent them: `Parameters` (read `query` as
text), decoding views (`query(decode)`, `path_text`), `Uri.fromParts`,
`setQuery`, `globParts`/`isRecursiveGlob`/`matchesGlobUnder`,
`partitionsUnder`/`withPartition`/`isPartitioned`, `joinPath` with an OS path
(`join_path`), `isLocal`, `localMimeType`, `defaultPort`, `port`, and the
`Scheme` type (`scheme` is a string).

## Gotchas in JavaScript

- `Uri.from(...)` always answers a `Uri`; narrow with `intoUrl()` /
  `intoUrn()` / `intoArn()` before using `Url`-only members (`parts`,
  `parent`, `match`, `partitions`, `relativeTo`).
- `Url.from(text)` / `new Url(text)` root relative text at `process.cwd()`;
  `Url.fromString(text)` refuses it. Store only the strict, absolute form.
- `query` and `fragment` are getters of the stored (escaped) text; decode
  with `decodeURIComponent` only for display, never to rebuild structure.
- Compare with `equals()` / `compare()`, not `===` or `toString()` equality
  of raw input; `stableHash()` is a `bigint`.
- `Url.joinpath` is spelled lowercase and `Uri.joinPath` camelCase; both are
  variadic.
