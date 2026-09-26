# yggdryl-storage in JavaScript

`const { IOBase, gzip, zlib, zstd, charset } = require('yggdryl')`. One
`IOBase` class covers every backend (memory, local, object stores, handler
filesystems); byte methods take and return `Buffer`/`Uint8Array`.

## Open a handle for a path or URL

`new IOBase(value)` / `IOBase.from(value)` accept a path, URL text, a `Url`,
`Uri`, `Urn` or `Arn`, or another handle. Nothing is opened, created or read.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { IOBase, Url } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))

// Construction probes nothing; a write creates the file and its parents.
const handle = new IOBase(path.join(root, 'nested', 'ticks.csv'))
assert.equal(handle.kind, 'unknown')
handle.writeText('symbol,price\nAAPL,1\n')
assert.equal(handle.kind, 'file')
assert.equal(handle.mediaType.toString(), 'text/csv')
assert.equal(handle.url.scheme, 'file')

// The same location through a URL value.
const again = IOBase.from(Url.fromPath(path.join(root, 'nested', 'ticks.csv')))
assert.equal(again.readText(), 'symbol,price\nAAPL,1\n')

fs.rmSync(root, { recursive: true, force: true })
```

## Hold bytes in memory

`IOBase.fromBytes` holds an in-memory buffer; its media type is sniffed, so
declare one the bytes cannot prove.

```javascript
const assert = require('node:assert/strict')
const { IOBase, MimeType } = require('yggdryl')

const sniffed = IOBase.fromBytes(Buffer.from('{"symbol":"AAPL"}'))
assert.ok(sniffed.mediaType.base.equals(MimeType.JSON))
assert.equal(sniffed.kind, 'memory')
assert.equal(sniffed.uri.scheme, 'mem')

const csv = IOBase.fromBytes(Buffer.from('symbol,price\n'))
assert.equal(csv.mediaType.toString(), 'application/octet-stream')
csv.mediaType = 'text/csv'
assert.equal(csv.mediaType.toString(), 'text/csv')
```

## Read a range, write at an offset, append

`readRangeBytes` transfers only the window and is clamped at the end;
`appendBytes` answers the offset the bytes landed at. `readRange(o, n, { text: true })`
and `append(string)` are the inferring doors over them.

```javascript
const assert = require('node:assert/strict')
const { IOBase } = require('yggdryl')

const handle = IOBase.fromBytes()
handle.writeBytes(Buffer.from('symbol,price\n'))
assert.equal(handle.appendBytes(Buffer.from('AAPL,1\n')), 13)

assert.equal(handle.readRangeBytes(13, 4).toString(), 'AAPL')
// A footer is one ranged read off the size (a getter), never the whole value.
assert.equal(handle.readRangeBytes(handle.size - 7, 7).toString(), 'AAPL,1\n')
assert.equal(handle.readRangeBytes(100, 4).length, 0) // past the end is empty
assert.equal(handle.readRange(0, 6, { text: true }), 'symbol')

// A write past the end zero-fills the gap.
handle.pwrite(22, Buffer.from('!'))
assert.equal(handle.size, 23)
assert.deepEqual([...handle.readRangeBytes(20, 3)], [0, 0, 0x21])

assert.equal(handle.append('MSFT'), 23)
assert.ok(handle.readText().endsWith('!MSFT'))
```

## Stream bounded chunks and use a cursor

`pstreamBytes(position, batchSize)` asks the core for one bounded chunk per
`next()` (64 KiB by default). A cursor shares the handle and owns one position.

```javascript
const assert = require('node:assert/strict')
const { IOBase } = require('yggdryl')

const handle = IOBase.fromBytes(Buffer.from('0123456789'))
assert.deepEqual([...handle.pstreamBytes(2, 3)].map(String), ['234', '567', '89'])

const cursor = handle.cursor(1)
assert.equal(cursor.streamBytes(2).next().value.toString(), '12')
assert.equal(cursor.tell(), 3)
cursor.seek(7)
assert.equal(cursor.read().toString(), '789')
assert.equal(cursor.position, 10)

const writer = IOBase.fromBytes()
writer.cursor().write(Buffer.from('symbol,price\n'))
assert.equal(writer.readText(), 'symbol,price\n')
```

## Decide a role: file, folder, or not yet

A location is `unknown` until something is there: a byte write settles it as
a file, `mkdir()` as a folder (parents included, existing left alone).

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { IOBase } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))

// Absence reads as empty, and nothing is created by asking.
const absent = new IOBase(path.join(root, 'sub', 'inner.bin'))
assert.equal(absent.exists(), false)
assert.equal(absent.size, 0)
assert.equal(absent.readBytes().length, 0)
assert.equal(fs.existsSync(path.join(root, 'sub')), false)

const day = new IOBase(path.join(root, 'day=2026-08-16'))
day.mkdir()
assert.equal(day.kind, 'directory')
assert.ok(day.isDir())

// A folder holds no bytes: byte writes are refused.
assert.throws(() => day.pwrite(0, Buffer.from('x')), /got the directory/)

const leaf = new IOBase(path.join(root, 'empty.bin'))
leaf.touch()
assert.ok(leaf.isFile())

fs.rmSync(root, { recursive: true, force: true })
```

## Walk, glob, clear and remove a tree

`joinpath` takes one slash path; listings (`ls`, `iterdir`, `glob`, `rglob`,
and iterating the handle) are lazy, sorted, and skip dot-names unless asked.
`clear` empties and keeps; `remove` deletes and treats absence as success.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { IOBase } = require('yggdryl')

const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-')), 'lake')
const lake = new IOBase(root)
for (const year of ['2024', '2025']) {
  lake.joinpath(`year=${year}/month=01/part-0.bin`).writeBytes(Buffer.from('x'))
}
fs.mkdirSync(path.join(root, '.git'))

assert.deepEqual([...lake].map((entry) => entry.name), ['year=2024', 'year=2025'])
assert.equal([...lake.ls(false, true)].length, 3)
assert.equal([...lake.ls(true)].length, 6)
assert.equal([...lake.glob('year=2024/**/*.bin')].length, 1)
assert.equal([...lake.rglob('*.bin')].length, 2)

const first = lake.joinpath('year=2024')
first.clear()
assert.equal([...first.iterdir()].length, 0)
assert.throws(() => lake.remove(), /children/)
lake.remove(true)
lake.remove(true) // absent: a no-op success
assert.equal(fs.existsSync(root), false)
```

## Media type and coding from the name

`codec` is the coding the name declares (`null` for none). JavaScript byte
methods (`readBytes`, `writeBytes`, `readText`, `writeText`) address the
**stored** bytes; the coding is applied by `readScalar`/`writeScalar`, the
record surface and `compressInto`/`decompressInto`.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { IOBase } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
const document = new IOBase(path.join(root, 'trade.json.gz'))
assert.equal(document.codec, 'gzip')
assert.equal(new IOBase(path.join(root, 'plain.csv')).codec, null)

// The value surface codes; the byte surface shows what is stored.
document.writeScalar({ quantity: 2, symbol: 'AAPL' })
assert.deepEqual([...document.readBytes().subarray(0, 2)], [0x1f, 0x8b])
assert.deepEqual(document.readScalar(), { quantity: 2, symbol: 'AAPL' })

fs.rmSync(root, { recursive: true, force: true })
```

## Compress or decompress into another handle

`compressInto(target, codec?, level?)` encodes every byte into `target`,
reading the codec off the target's name; an in-memory target names it
explicitly. `decompressInto(target)` reads the source's declared coding.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { IOBase } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
const plain = new IOBase(path.join(root, 'rows.json'))
plain.writeBytes(Buffer.from('{"symbol":"AAPL"}'))

const encoded = new IOBase(path.join(root, 'rows.json.gz'))
assert.equal(plain.compressInto(encoded), encoded.size)
assert.deepEqual([...encoded.readBytes().subarray(0, 2)], [0x1f, 0x8b])

const decoded = IOBase.fromBytes()
assert.equal(encoded.decompressInto(decoded), 17)
assert.equal(decoded.readText(), '{"symbol":"AAPL"}')

// A nameless target declares no coding: name one.
const memory = IOBase.fromBytes()
assert.ok(plain.compressInto(memory, 'zstd', 9) > 0)
assert.equal(memory.codec, 'zstd')

// A target declaring no coding is refused rather than copied unchanged.
assert.throws(
  () => plain.compressInto(new IOBase(path.join(root, 'copy.json'))),
  /expected a target declaring a content coding/,
)
assert.equal(Number(plain.copyInto(new IOBase(path.join(root, 'copy.json')))), 17)

fs.rmSync(root, { recursive: true, force: true })
```

## Compress bytes without a handle

`gzip`, `zlib` and `zstd` are frozen `loads`/`dumps` namespaces over whole
`Buffer`s, wire-compatible with `node:zlib`; `dumps(data, level?)` takes the
shared 0-9 scale. `zlib` adds `loadsRaw`/`dumpsRaw` for raw DEFLATE.

```javascript
const assert = require('node:assert/strict')
const standard = require('node:zlib')
const { gzip, zlib, zstd } = require('yggdryl')

const payload = Buffer.from('symbol,price\n' + 'AAPL,1\n'.repeat(64))

const encoded = gzip.dumps(payload, 9)
assert.deepEqual(gzip.loads(encoded), payload)
assert.deepEqual(standard.gunzipSync(encoded), payload)

const raw = zlib.dumpsRaw(payload)
assert.deepEqual(zlib.loadsRaw(raw), payload)
assert.equal(zlib.dumps(payload).length, raw.length + 6)
assert.deepEqual(standard.inflateRawSync(raw), payload)
assert.throws(() => zlib.loads(raw))

const frame = zstd.dumps(payload)
assert.ok(frame.length < payload.length)
assert.deepEqual(zstd.loads(frame), payload)
```

## Decode a charset once

`charset.decode(name, bytes)` answers what `new TextDecoder(name)` answers,
with the crate's refusal; `encode` is the direction `TextEncoder` lacks. A
whole resource is decoded by declaring the charset on its media type; record
reads then arrive as strings. `readText` is strict UTF-8.

```javascript
const assert = require('node:assert/strict')
const { IOBase, charset } = require('yggdryl')

const wire = Buffer.from('prix: 12\x80', 'latin1')
assert.equal(charset.decode('windows-1252', wire), 'prix: 12€')
assert.deepEqual(charset.encode('windows-1252', 'prix: 12€'), wire)
assert.equal(charset.canonicalName('cp1252'), 'windows-1252')

assert.throws(() => charset.decode('us-ascii', Buffer.from([0x63, 0xe9])), /us-ascii/)
assert.equal(charset.decodeLossy('us-ascii', Buffer.from([0x63, 0xe9])), 'c�')

assert.deepEqual(charset.fromBom(Buffer.from([0xef, 0xbb, 0xbf, 0x41])), { charset: 'utf-8', length: 3 })
assert.equal(charset.bom('windows-1252'), null)

const handle = IOBase.fromBytes(Buffer.from('Zürich\n', 'latin1'))
handle.mediaType = 'text/plain;charset=windows-1252'
assert.equal(handle.mediaType.charset, 'windows-1252')
assert.deepEqual([...handle.readRecords()].map((row) => row.body), ['Zürich'])
```

## Digest or parse the value a handle holds

`readDigest(algorithm?)` streams and holds one chunk (`'xxh3-64'` by default);
`readScalar(field)` picks JSON/YAML/TOML/XML and any outer coding from the
media type; `{ field, scalar: true }` answers the core `Scalar`.

```javascript
const assert = require('node:assert/strict')
const { IOBase, Scalar, xxhash } = require('yggdryl')

const payload = Buffer.from('symbol,price\nAAPL,1\n')
const handle = IOBase.fromBytes(payload)
assert.ok(handle.readDigest().equals(xxhash.digest(payload, 'xxh3-64')))
assert.ok(handle.readRangeDigest(0, 6, 'xxh32').equals(xxhash.digest(Buffer.from('symbol'), 'xxh32')))

const document = IOBase.fromBytes()
document.mediaType = 'application/json'
document.writeScalar({ quantity: 2, symbol: 'AAPL' })
const field = 'trade: struct<quantity: int32 not null, symbol: utf8 not null> not null'
assert.deepEqual(document.readScalar(field), { quantity: 2, symbol: 'AAPL' })
const value = document.readScalar({ field, scalar: true })
assert.ok(value instanceof Scalar)
assert.equal(value.kind, 'serie')
```

## Repeat reads: open a scope or add a page cache

`open()` moves materialization to a known point and keeps caches until
`close()`; `buffered({ pageSize, maxBytes, ttlMs })` adds a page cache to the
same handle and returns it.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { IOBase } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
const target = path.join(root, 'value.bin')
fs.writeFileSync(target, 'symbol,price\n')

const handle = new IOBase(target)
handle.open()
assert.equal(handle.opened(), true)
assert.equal(handle.readRangeBytes(0, 6).toString(), 'symbol')
handle.close()
assert.equal(handle.closed(), true)

const cached = IOBase.fromBytes(Buffer.alloc(16 * 64))
assert.equal(cached.buffered({ pageSize: 64, maxBytes: 4 * 64, ttlMs: 30_000 }), cached)
cached.readRangeBytes(16 * 64 - 8, 8)
assert.equal(cached.readRangeBytes(0, 8).length, 8)

fs.rmSync(root, { recursive: true, force: true })
```

## Name an object on S3, Google Cloud Storage or Azure

The scheme picks the store; construction, children, parents, media types and
partitions make no request - the first read or write does, with credentials
resolved the way each store's own tools resolve them.

```javascript
const assert = require('node:assert/strict')
const { IOBase, Arn } = require('yggdryl')

const part = new IOBase('s3://trades/lake/year=2026/part.parquet')
assert.equal(part.url.bucket, 'trades')
assert.equal(part.url.key, 'lake/year=2026/part.parquet')
assert.deepEqual(part.partitions, [{ column: 'year', value: '2026' }])
assert.equal(part.mediaType.toString(), 'application/vnd.apache.parquet')

const blob = new IOBase('abfss://lake@trades.dfs.core.windows.net/part.bin')
assert.equal(blob.url.bucket, 'lake')
assert.equal(blob.parent.url.toString(), 'abfss://lake@trades.dfs.core.windows.net/')

// An Amazon S3 ARN opens the s3: URL it names.
assert.equal(new IOBase(new Arn('arn:aws:s3:::b/k')).url.toString(), 's3://b/k')
```

## Bind a filesystem through the handler protocol

`IOBase.fromFs(handler, path, uri?)` wraps any object answering sixteen
synchronous calls (sizes, offsets and nanosecond mtimes are `bigint`). The
path is opaque: `%2F` reaches the handler literally.

```javascript
const assert = require('node:assert/strict')
const { IOBase } = require('yggdryl')

const files = new Map()
const handler = {
  typeName: 'memory',
  equals: (other) => other === handler,
  normalizePath: (p) => p,
  fileInfo: (p) =>
    files.has(p) ? { path: p, kind: 'file', size: BigInt(files.get(p).length) } : { path: p, kind: 'not-found' },
  *list(selector) {
    for (const [p, bytes] of files) {
      if (p.startsWith(selector.baseDir)) yield { path: p, kind: 'file', size: BigInt(bytes.length) }
    }
  },
  createDir() {},
  deleteDir: (p) => void files.delete(p),
  deleteDirContents: () => files.clear(),
  deleteRootDirContents: () => files.clear(),
  deleteFile: (p) => void files.delete(p),
  copyFile: (source, target) => void files.set(target, files.get(source)),
  move(source, target) {
    files.set(target, files.get(source))
    files.delete(source)
  },
  openInputFile(p) {
    const bytes = files.get(p) ?? new Uint8Array()
    let at = 0n
    return {
      closed: false,
      readAt: (offset, length) => bytes.slice(Number(offset), Number(offset) + Number(length)),
      seek(offset) {
        at = offset
        return at
      },
      read(length) {
        const out = bytes.slice(Number(at), Number(at) + Number(length))
        at += BigInt(out.length)
        return out
      },
      tell: () => at,
      close() {},
    }
  },
  openInputStream: (p) => handler.openInputFile(p),
  openOutputStream(p) {
    const chunks = []
    return {
      closed: false,
      write(bytes) {
        chunks.push(bytes)
        return BigInt(bytes.length)
      },
      tell: () => BigInt(chunks.reduce((sum, one) => sum + one.length, 0)),
      flush() {},
      close: () => void files.set(p, Buffer.concat(chunks)),
    }
  },
  openAppendStream: (p) => handler.openOutputStream(p),
}

const source = IOBase.fromFs(handler, 'bucket/v=a%2Fb.bin')
source.writeBytes(Buffer.from('trades'))
assert.deepEqual([...files.keys()], ['bucket/v=a%2Fb.bin'])
assert.equal(source.readRangeBytes(1, 3).toString(), 'rad')

const target = IOBase.fromFs(handler, 'archive/v=a%2Fb.bin')
source.copyInto(target)
assert.equal(Buffer.from(target.readBytes()).toString(), 'trades')
```

## Rust only

Not bound in JavaScript - never invent these: a decoded-view handle
(`into_coded`/`Coded`; use `compressInto`/`decompressInto` or the codec
namespaces), `Transcoded`, `Counted` call tallies, ZIP archives, role classes
(`LocalFile`, `LocalFolder`, `S3File`), `LocalFolder.temporary()/home()`,
streaming codec readers/writers, `S3Options`/`Session` builders. JavaScript
has no S3 client for `IOBase.fromUri('s3://...')` (it reports `Unsupported`);
`new IOBase('s3://...')` is the native store.

## Gotchas in JavaScript

- `writeBytes`/`writeText` on a `.gz` name write **plain** bytes: the byte
  surface is the stored bytes. Code with `compressInto`, `gzip.dumps`, or the
  value/record surfaces.
- `joinpath('a', 'b.bin')` joins segment by segment and needs `a` to be a
  container already; `joinpath('a/b.bin')` descends in one step and creates
  parents on write.
- `size`, `kind`, `mediaType`, `codec`, `parent`, `partitions`, `url` are
  getters; `opened()`, `closed()`, `exists()`, `isDir()`, `isFile()` are methods.
- `copyInto` answers a `bigint`; `compressInto`/`decompressInto` a `number`.
- `buffered(...)` returns the same handle (Python's spends it).
- A handler-backed handle is bound to the JavaScript thread that supplied the
  handler; it cannot be read from a `Worker`.
- `gzip`, `zlib` and `zstd` exist at runtime but ship no TypeScript
  declarations in this version; `charset` does.
