# JavaScript

A Node-API view of the same values the Rust core holds, with conventional JavaScript casing and protocols.

```javascript
const { DataType, Field, Url } = require('yggdryl')
const assert = require('node:assert/strict')

const schema = new Field(
  'row',
  DataType.fromFields([new Field('id', 'int64', false)]),
  false,
)

assert.equal(schema.dataType.kind, 'struct')
assert.equal(String(Url.fromPath('C:/market data/trades.arrows')),
  'file:///C:/market%20data/trades.arrows')
```

This page documents the JavaScript boundary only: what the package adds on top of the core, and how
it converts what you hand it. The behaviour itself is documented once, on the
[core pages](../index.md).

## Build from the repository

```console
npm install --prefix node
npm run --prefix node build:debug
npm test --prefix node
```

`npm test` runs `node --test tests/*.test.js` and then `tsc --noEmit`, so the shipped `.d.ts`
declarations are checked against the tests that use them.

## What it exposes

| Name | Documented in |
| --- | --- |
| `DataType` | [datatype](../datatype.md) |
| `Field`, `fields` | [field](../field.md) |
| `Expression`, `Bound`, `Statement`, `BoundStatement` | [expression](../expression.md) |
| `Uri`, `Url`, `Urn` | [uri](../uri.md) |
| `IOBase` | [io](../io.md) |
| `BatchReader`, `RecordOptions` | [io](../io.md), [ipc](../ipc.md), [parquet](../parquet.md) |
| `fieldFromPattern` | [io](../io.md) |
| `iceberg` | [iceberg](../iceberg.md) |
| `MimeType`, `MediaType`, `Timezone` | [enums](../generic.md) |
| `codec`, `json`, `toml`, `yaml`, `Scalar` | [text](../text.md) and the format pages |
| `avro` | [Avro](../avro.md) schema, container, single-object, and batch media |
| `gzip`, `zlib`, `zstd` | [gzip](../gzip.md), [zlib](../zlib.md), [zstd](../zstd.md) |

Each coding namespace carries the whole-buffer pair - `loads` and `dumps`, plus `loadsRaw` and
`dumpsRaw` on `zlib` - over `Buffer`, reading and writing exactly what `node:zlib` does. Their
streaming `reader`/`writer` and the transparent handle wrappers stay Rust-only: both are built on
Rust's `Read`/`Write`, which has no JavaScript spelling here. A handle still applies the coding its
name declares without being told, and `IOBase.codec` is what asks it which one that is.

## A filesystem is whatever answers six calls

Arrow JS ships no filesystem, so where Python hands the core a `pyarrow.fs.FileSystem` that already
exists, JavaScript supplies the vtable itself: a plain object whose methods are Arrow's own
`FileSystem` calls in camelCase - `fileInfo`, `list`, `readRange`, `writeFull`, `createDir`,
`deleteFile`, plus a `typeName`. `IOBase.fromArrowFs(handler, path)` turns one into an ordinary
handle, so a `Map`, `node:fs`, an S3 client, or a caching layer over any of them becomes storage
the rest of the package can read and write. [`arrowfs`](../arrowfs.md) documents the backend and
shows a complete handler.

Two things belong to this boundary rather than to the backend. Sizes cross as `bigint`, because a
64-bit length is what an object store reports and a JavaScript number cannot hold one exactly - an
exact `number` is accepted, since a handler over `fs.Stats` already has one. And the handler is
called **synchronously, on the thread that supplied it**: Node-API's only cross-thread call is
asynchronous, while every method here has to answer in the middle of a native read, so a
handler-backed handle refuses a `Worker` by name instead of pretending. A worker that needs its own
view builds its own handler; only the location string has to travel.

`using` binds to `open`/`close`, which is what publishes a staged whole value - the shape every
Arrow filesystem writes in.

## Inference at the boundary

Every constructor accepts the obvious JavaScript spelling of its argument and converts once, in
Rust. Prefer the generic `from` entry points: they dispatch on what they were handed.

```javascript
const { DataType, Field, MediaType, MimeType, Uri, Url } = require('yggdryl')
const assert = require('node:assert/strict')

// A datatype expression is a datatype.
assert.equal(String(new Field('id', 'int64', false).dataType), 'int64')
assert.equal(DataType.from('list<int32>').kind, 'list')

// A media type is its canonical name.
assert.equal(String(MimeType.from('application/json')), 'application/json')
assert.equal(String(MediaType.from('application/json')), 'application/json')

// A path is a location.
assert.equal(String(Url.fromPath('C:/tmp/a.json')), 'file:///C:/tmp/a.json')
assert.equal(
  String(Uri.fromString('s3://warehouse/db').joinPath('trades', 'data.parquet')),
  's3://warehouse/db/trades/data.parquet',
)
```

There is no JavaScript-side parser: `DataType.from` and its siblings call the matching native
constructor. JavaScript has no object `/` operator protocol, so generic URI path
composition uses variadic `Uri.joinPath`; every component is normalized by the
same core `joinpath` implementation as Rust's and Python's `/` idiom.

## Bytes and ranges

The core's byte methods keep their exact names, so `readRangeBytes` and `appendBytes` are
[the core calls](../io.md#whole-values) spelled in JavaScript. Over each sits one inferring entry
point that coerces at the boundary and redirects to that same native: `readRange` chooses the
answer's type from its options, and `append` chooses how to read the byte source it was handed.

```javascript
const assert = require('node:assert/strict')
const { IOBase } = require('yggdryl')

const handle = IOBase.fromBytes(Buffer.from('symbol,price\n'))

// The explicit core names.
assert.equal(handle.appendBytes(Buffer.from('AAPL,1\n')), 13)
assert.deepEqual(handle.readRangeBytes(0, 6), Buffer.from('symbol'))

// `{ text: true }` selects the answer's type; omitting it answers a Buffer.
assert.deepEqual(handle.readRange(0, 6), Buffer.from('symbol'))
assert.equal(handle.readRange(0, 6, { text: true }), 'symbol')

// `append` also takes a string, a plain Uint8Array, and an ArrayBuffer.
assert.equal(handle.append('MSFT,2\n'), 20)
assert.equal(handle.append(new Uint8Array([78, 86, 68, 65, 10])), 27)
assert.equal(handle.append(Uint8Array.from([73, 78, 84, 67, 10]).buffer), 32)
assert.equal(handle.readRange(20, 7, { text: true }), 'MSFT,2\n')
```

`readRange` rejects an unknown option and a non-boolean `text`, the way `readScalar` rejects its
own. `append` reads a `Uint8Array` - a `Buffer` is one - an `ArrayBuffer`, and a string, encoding
text as UTF-8 exactly as `writeText` does, and returns the byte offset the append landed at.

## One native field from a class or value

JavaScript follows the [canonical field-conversion contract](../field.md#converting-to-one-native-field)
with `intoField(value, name?)` and the class-level `intoStructField` spelling. The canonical class
form is an actual static getter; `intoField` validates its native non-null Struct result and memoizes
that result per class. Other class member forms are rejected.

```javascript
const assert = require('node:assert/strict')
const { fields, intoField } = require('yggdryl')

let builds = 0
class Quote {
  static get intoStructField() {
    builds += 1
    return fields.struct('Quote', [fields.int64('id')], { nullable: false })
  }
}

const root = intoField(Quote)
assert.strictEqual(intoField(new Quote()), root)
assert.equal(builds, 1)

const renamed = intoField(Quote, 'quote')
assert.equal(renamed.name, 'quote')
assert.equal(root.name, 'Quote')
```

The converter also accepts a native `Field` or field expression. Invalid roots are rejected before
they cross into a table operation: `intoStructField` must resolve to a native Struct field whose
root is non-null.

## Scalars cross as their natural shape

A JavaScript value becomes the nearest native scalar, and comes back as the nearest JavaScript value
to that. No class name travels beside the data, so a shape the core does not have arrives as the
shape it was lowered to.

```javascript
const { yaml } = require('yggdryl')
const assert = require('node:assert/strict')

const decoded = yaml.loads(yaml.dumps({
  venues: new Set(['XPAR', 'XNAS']),
  book: new Map([[1, 'bid']]),
  source: new URL('https://example.com/feed'),
  match: /a\/b/giu,
  raw: Buffer.from([0, 255]),
  id: 2n ** 100n,
}))

assert.deepEqual(decoded.venues, ['XPAR', 'XNAS'])        // a Set is a list
assert.ok(decoded.book instanceof Map)                    // a non-text key keeps the Map
assert.equal(decoded.source, 'https://example.com/feed')  // a URL is its href
assert.equal(decoded.match, '/a\\/b/giu')                 // a RegExp is its literal
assert.deepEqual(decoded.raw, Buffer.from([0, 255]))
assert.equal(decoded.id, 2n ** 100n)
```

| You write | It is stored as | It reads back as |
| --- | --- | --- |
| `undefined`, `null` | null | `null` |
| `boolean`, `string` | boolean, string | the same |
| `number` | 64-bit integer or float | `number` |
| `bigint` | exact 64- or 128-bit integer | `number` inside the safe range, `bigint` outside |
| `Buffer`, `Uint8Array`, `Uint8ClampedArray`, `ArrayBuffer` | bytes | `Buffer` |
| every other typed array | sequence | `Array` |
| `Array`, `Set` | sequence | `Array` |
| `Map` | mapping | `Map` when some key is not text, plain object when every key is |
| plain object, class instance | sorted `Record` | plain object |
| `Date` | `DateTime64(ms, UTC)` | `Date` |
| `URL` | its `href` string | `string` |
| `RegExp` | its literal string, flags included | `string` |
| `DataType`, `Field` | core structural mapping | plain object |
| `Uri`, `Url`, `Urn` | canonical string | `string` |
| `Scalar` | itself | `Date` when one holds it exactly, otherwise `Scalar` |

These are the losses, and they are deliberate: a `Set` comes back a list, a `URL` and a `RegExp`
come back strings, a class instance comes back a plain object, a `Map` of text keys comes back a
plain object, and `undefined` comes back `null`. Reconstructing any of them takes one line of your
own code and needs no cooperation from the codec:

```javascript
const { yaml } = require('yggdryl')
const assert = require('node:assert/strict')

class Order {
  constructor(id) {
    this.id = id
  }
}

const decoded = yaml.loads(yaml.dumps({ order: new Order(7), venues: new Set(['XPAR']) }))
assert.deepEqual(decoded, { order: { id: 7 }, venues: ['XPAR'] })

const order = Object.assign(new Order(0), decoded.order)
assert.ok(order instanceof Order)
assert.deepEqual(new Set(decoded.venues), new Set(['XPAR']))
```

Nothing in a document can make this binding name a class, look one up, or run a constructor,
because there is no name in the document to look up. A `bigint` wider than 128 bits has no exact
native integer and is refused rather than rounded, and reading back a `Date`, a `Buffer`, or a
`Map` never depends on a method the caller can replace: the encoder reads those off the intrinsic
prototypes.

## Scalar families

`Scalar.float(value, width = 64)` selects 16, 32, or 64 bits.
`decimal(coefficient, scale = 0)` selects the narrowest exact decimal. Date and
time widths follow the unit; duration follows the count; Arrow datetime is
always 64-bit. Exact `kind` values survive field-directed transport.

Temporal factories use `(count, unit, timezone)`. `date` defaults to days; an
omitted timezone becomes `NAIVE`. Time and duration accept only `NAIVE`, while
datetime also accepts a timezone name or `Timezone`. Schemaless text writes
natural ISO strings; `loads(..., { field })` restores the declared exact type.

```javascript
const { Timezone, Scalar, json } = require('yggdryl')
const assert = require('node:assert/strict')

const price = Scalar.decimal(-(2n ** 200n), 7)
assert.equal(price.kind, 'd256')
assert.equal(price.scale, 7)
assert.ok(Scalar.decimal(150n, 2).equals(Scalar.decimal(15n, 1)))

const at = Scalar.datetime(1700000000123456n, 'us', 'UTC')
assert.equal(json.loads(json.dumps(at)), '2023-11-14T22:13:20.123456Z')
assert.equal(Scalar.datetime(0n, 'ms').zone, 'NAIVE')
assert.equal(Scalar.time(1n, 'us', Timezone.from('NAIVE')).zone, 'NAIVE')
assert.ok(
  Scalar.fromJs(new Date('2026-08-15T12:30:00.000Z'))
    .equals(Scalar.datetime(1786797000000n, 'ms', 'UTC')),
)
```

`kind`, `count`, `unit`, `zone`, `unscaled`, and `scale` expose the matching
payload. `asBytes`/`asUtf8` borrow scalar content; `asJsonBytes` and
`asJsonUtf8` use the core natural JSON writer.

## Native value protocols and checked arithmetic

Immutable native values expose `equals`, `compare`, `stableHash`, and `clone`
when the Rust core has a complete value identity. This includes `Scalar`,
`Expression`, `Statement`, `avro.Schema`, `iceberg.PartitionSpec`,
`iceberg.DataFile`, and `iceberg.ScanPlan`; other domain wrappers expose the
same methods wherever their core identity is complete. `compare` uses the
core's total order, and equal values always return the same deterministic
`stableHash()` `bigint`. JavaScript has no object hash protocol, so
`stableHash()` is explicit and `===` remains reference identity. Operational
readers, iterators, and handles do not invent a value identity from hidden
state.

`Scalar` equality, order, and hashing normalize equivalent decimal and temporal
resolutions. Avro schema identity retains logical types, defaults, aliases, and
extension attributes; `fingerprint()` alone follows Parsing Canonical Form.
`iceberg.IcebergOptions` has the same four methods over its current explicit
configuration, so a detached clone stops comparing equal after either copy is
mutated.

JavaScript cannot overload arithmetic operators, so `Scalar` exposes the
checked core operations as `add`, `subtract`, `multiply`, `divide`,
`remainder`, `negate`, and `absolute`. Each binary method accepts either a
native `Scalar` or a JavaScript value inferred once through `Scalar.fromJs`, and
returns a native `Scalar`. Numeric operations preserve widths and promote only
as the core defines; exact decimal division uses the smallest terminating
scale. Supported datetime/duration pairs use the same checked path. Text and
containers are never silently concatenated or coerced.

`Expression` exposes `add`, `subtract`, `multiply`, `divide`, `remainder`, and
`negate` as lazy tree builders. An `Expression` operand stays native, a string
is parsed as expression text, and any other JavaScript value is inferred once
as a literal.

```javascript
const assert = require('node:assert/strict')
const { Expression, Scalar } = require('yggdryl')

const half = Scalar.decimal(1n).divide(Scalar.decimal(2n))
assert.ok(half.equals(Scalar.decimal(5n, 1)))
assert.equal(half.clone().compare(half), 0)
assert.equal(typeof half.stableHash(), 'bigint')

const size = Expression.column('size').add(1)
assert.equal(size.toString(), 'size + 1')
assert.ok(size.clone().equals(size))

assert.throws(
  () => Scalar.fromJs(1).divide(0),
  (error) => error instanceof RangeError &&
    error.code === 'ERR_YGGDRYL_DIVISION_BY_ZERO',
)
```

Invalid operand kinds throw `TypeError` with
`ERR_YGGDRYL_INVALID_ARITHMETIC`. Overflow, division by zero, and non-terminating
exact decimal division throw `RangeError` with distinct
`ERR_YGGDRYL_ARITHMETIC_OVERFLOW`, `ERR_YGGDRYL_DIVISION_BY_ZERO`, and
`ERR_YGGDRYL_INEXACT_ARITHMETIC` codes.

## fromJs and asJs

`Scalar.fromJs` and `Scalar.prototype.asJs` are the conversion pair. Every `load` and `dump` crosses
them - `dumps` is `fromJs` with bytes on the far side, `loads` is `asJs` - so calling them directly
is how you see what a value becomes before any format is involved.

```javascript
const { Scalar, json } = require('yggdryl')
const assert = require('node:assert/strict')

assert.equal(Scalar.fromJs(new Set([1, 2])).kind, 'sequence')
assert.deepEqual(Scalar.fromJs(new Set([1, 2])).asJs(), [1, 2])
assert.equal(Scalar.fromJs(new Map([['id', 1]])).kind, 'mapping')

const value = { id: 1, tags: new Set(['a']) }
assert.deepEqual(json.loads(json.dumps(value)), Scalar.fromJs(value).asJs())

const tree = Scalar.fromJs({ legs: [{ id: 1 }] })
assert.equal(tree.get('legs').at(0).get('id').asJs(), 1)
assert.equal(tree.set('venue', 'XNAS').get('venue').asUtf8(), 'XNAS')
```

`length`, iteration, `at`, `get`, `has`, and `path` return exact native child
values; `set` and `remove` return a rebuilt value without mutating the source.
`fromJs` and `asJs` accept nullable `{ maxDepth }` for language traversal in
the inclusive range 1 to 48. Input-byte, node, and document limits apply to
codec loads, where a text parser actually runs.

## Field metadata is a Map

`Field` implements the `Map` protocol over its metadata, so ordinary idioms work and the ordering is
the native one.

```javascript
const { Field } = require('yggdryl')
const assert = require('node:assert/strict')

const field = new Field('trade', 'int64', false, { source: 'book' })
field.set('venue', 'XPAR')

assert.equal(field.get('source'), 'book')
assert.ok(field.has('venue'))
assert.equal(field.size, 2)
assert.deepEqual([...field.keys()].sort(), ['source', 'venue'])

field.delete('venue')
assert.ok(!field.has('venue'))
```

Typed identifiers and typed HTTP values (`dictionaryId`, `contentType`, `etag`, and the rest) are
accessors rather than map keys, because they are validated.

One protocol's properties are a `Map` of their own, and it is a live view of the same field rather
than a copy of part of it.

```javascript
const { Field } = require('yggdryl')
const assert = require('node:assert/strict')

const field = new Field('price', 'int64', false)
field.iceberg.set('doc', 'closing price')
field.postgres.update({ type: 'numeric' })

assert.equal(field.iceberg.get('doc'), 'closing price')
assert.deepEqual([...field.postgres], [['type', 'numeric']])
assert.equal(field.iceberg.size, 1)
assert.equal(field.postgres.has('doc'), false)

// The bare name is all the view needs; the full key is what the field stores.
assert.equal(field.iceberg.key('doc'), 'iceberg:doc')
assert.equal(field.get('iceberg:doc'), 'closing price')
assert.equal(field.size, 2)

assert.equal(field.iceberg.delete('doc'), true)
assert.equal(field.iceberg.size, 0)
```

Every well-known protocol is a getter - `iceberg`, `postgres`, `http`, `arrow`, `spark`, `s3`, and
the rest - and `field.protocol(name)` takes one that is only known at runtime. There is no `https`
getter, because HTTPS shares the canonical `http:` namespace.

A schema also says which of its columns a path spells out, which is what a partitioned write and an
Iceberg spec both read.

```javascript
const { DataType, Field } = require('yggdryl')
const assert = require('node:assert/strict')

const schema = new Field(
  'row',
  DataType.fromFields([
    new Field('year', 'int32', false),
    new Field('price', 'int64', false),
  ]),
  false,
).withPartitionFields(['year'])

assert.deepEqual(schema.partitionFieldNames(), ['year'])
assert.equal(schema.dataType.getByName('year').isPartition, true)
assert.equal(schema.withoutPartitionFields().dataType.length, 1)
```

## Arrow

Apache Arrow JS values cross the boundary as copied IPC. The package is explicit about that: this is
not a zero-copy bridge.

```javascript
const { DataType } = require('yggdryl')
const assert = require('node:assert/strict')

const scalar = DataType.from('int64').defaultArrowScalar()
assert.equal(String(scalar), '0')
```

`defaultJSValue`, `defaultJSHint`, and `defaultArrowScalar` are schema-directed projections of the
native default planner; the JavaScript layer caches identity but never decides what a default is.

## Records: explicit intent and representation

`BatchReader` is the primitive record shape: `readArrowReader` returns one and each
`overwriteArrowReader`/`appendArrowReader`/`mergeArrowReader` call consumes one, exactly as
[`IOMedia`](../io.md) does in Rust. `BatchReader.from` accepts whatever a caller already holds -
another reader, an Apache Arrow JS `Table` or `RecordBatch`, an array of batches, or Arrow IPC bytes -
and iterating one yields Arrow JS record batches.

```javascript
const assert = require('node:assert/strict')
const arrow = require('apache-arrow')
const { BatchReader, IOBase, MimeType } = require('yggdryl')

const table = new arrow.Table({
  id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
})

// An in-memory handle says what it holds; a named one reads it off its name.
const handle = IOBase.fromBytes()
handle.mediaType = MimeType.ARROW_STREAM
handle.overwriteArrowReader(BatchReader.from(table))

const reader = handle.readArrowReader()
assert.equal(reader.field.name, 'row')
assert.equal([...reader].reduce((rows, batch) => rows + batch.numRows, 0), 2)
assert.equal(handle.isIo(), true)
assert.equal(handle.rowSize, 2)
assert.equal(handle.columnSize, 1)

// A stream is read once, and says so rather than reading as empty.
assert.ok(reader.consumed)
```

Each batch crosses as its own self-contained Arrow IPC stream, so its schema travels with it and
Arrow JS needs no separate handshake. That per-batch header is what a copied boundary costs, and it
is stated here rather than hidden. `intoIpc` drains the reader into one stream and `intoTable` into one
Arrow JS table, for the cases where a caller does want everything at once.

`isIo()` asks the core whether the handle exposes either its byte or record surface; a container
holding neither returns false. `rowSize` and `columnSize` describe the whole logical media, not the
last projection, filter, or row limit used to read it. They are lazy metadata getters: formats with
a cheap row count answer without decoding rows, successful values are cached while `open()` holds
the media metadata, a write through the handle invalidates them, and `close()` makes the next access
compute fresh. JavaScript stores no parallel schema or count. Counts beyond the exact integer range
saturate at `Number.MAX_SAFE_INTEGER`.

The encoding is never named by a call: `recordOptions()` derives it from the handle's media type, and
`RecordOptions` carries the shared settings plus the Parquet-only ones, which read as `null` on an
encoding that has none.

`RecordOptions` exposes `equals`, `compare`, `stableHash`, and `clone` by
delegating to the complete core value. Identity includes the encoding variant,
every shared setting, and every Avro, Parquet, IPC, or Text-specific setting.
For Text, declared regex source and extractor settings participate; compiled
regex caches and derived fields do not. A clone is detached, so changing it
does not mutate or re-hash the source options. JavaScript `===` remains object
identity and `stableHash()` is the explicit deterministic `bigint`.

```javascript
const assert = require('node:assert/strict')
const { Field, RecordOptions, fields } = require('yggdryl')

const parquet = RecordOptions.from('trades.parquet')
assert.equal(String(parquet.mimeType), 'application/vnd.apache.parquet')
assert.equal(parquet.compression, 'zstd(1)')
assert.equal(parquet.withCompression('snappy').compression, 'snappy')

const clone = parquet.clone()
assert.ok(clone.equals(parquet))
assert.equal(clone.compare(parquet), 0)
assert.equal(clone.stableHash(), parquet.stableHash())
clone.compression = 'snappy'
assert.ok(!clone.equals(parquet))

const root = fields.struct('row', [Field.from('id: int64')], { nullable: false })
const declared = parquet.withField(root)
assert.ok(declared.field.equals(root))

// A setting one encoding has is absent on the others rather than invented.
const stream = RecordOptions.from('trades.arrows')
assert.equal(stream.compression, null)
assert.equal(stream.maxRowGroupSize, null)
assert.equal(stream.commitRowSize, null)
assert.equal(stream.withCommitRowSize(10_000).commitRowSize, 10_000)
```

### Pick the shape you have and the intent you mean

The explicit method name carries both facts:

| Input | Overwrite | Append | Key-matched merge |
| --- | --- | --- | --- |
| native `BatchReader` | `overwriteArrowReader` | `appendArrowReader` | `mergeArrowReader` |
| Arrow JS `Table` | `overwriteArrowTable` | `appendArrowTable` | `mergeArrowTable` |
| Arrow JS `RecordBatch` | `overwriteArrowBatch` | `appendArrowBatch` | `mergeArrowBatch` |
| plain objects or field-class instances | `overwriteRecords` | `appendRecords` | `mergeRecords` |

When mode comes from configuration, each shape also has one dispatcher. The
canonical order is input, required mode, then the one optional settings value:

```text
writeArrowReader(reader, mode, options?)
writeArrowTable(table, mode, options?)
writeArrowBatch(batch, mode, options?)
writeRecords(records, mode, options?)
```

`mode` is `'overwrite'`, `'append'`, or `'merge'`. It is validated before a
one-shot reader, Arrow exporter, synchronous iterator, or asynchronous iterator
is touched. The dispatcher does not guess either fact: the method still names
the representation and the argument explicitly names intent.

The reader method accepts only a native `BatchReader`; use `BatchReader.from(value)` when you want
to explicitly convert Arrow IPC bytes or another supported Arrow representation. Table and batch
methods validate their named shape, infer a native `Field`, and redirect to the matching reader
primitive. Arrow JS has no C Data consumer, so those two adapters copy one IPC stream at the
boundary. That is the only materialization they add.

```javascript
const assert = require('node:assert/strict')
const arrow = require('apache-arrow')
const { BatchReader, IOBase, MimeType } = require('yggdryl')

const first = new arrow.Table({
  id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
  venue: arrow.vectorFromArray(['XNAS', 'XNYS'], new arrow.Utf8()),
})
const later = new arrow.Table({
  id: arrow.vectorFromArray([2n, 4n], new arrow.Int64()),
  venue: arrow.vectorFromArray(['XPAR', 'XTKS'], new arrow.Utf8()),
})
const extra = new arrow.Table({
  id: arrow.vectorFromArray([3n], new arrow.Int64()),
  venue: arrow.vectorFromArray(['XLON'], new arrow.Utf8()),
})

const handle = IOBase.fromBytes()
handle.mediaType = MimeType.ARROW_STREAM
handle.overwriteArrowTable(first)
handle.appendArrowReader(BatchReader.from(extra))

const merging = handle.recordOptions().withMergeByNames(['id'])
handle.mergeArrowTable(later, merging)
assert.equal(handle.readArrowReader().intoTable().numRows, 4)

// A configured mode reaches the same native dispatcher.
handle.writeArrowTable(extra, 'append')
```

Intent is authoritative. `mergeByNames` supplies keys only: overwrite and append reject it, while
merge requires it to be non-empty. This validation happens before a one-shot reader is consumed,
before a table or batch is IPC-encoded, and before a record iterable is pulled.

### Records and field classes

Plain objects infer one struct field through Arrow JS. A class instance whose constructor exposes a
static `intoStructField` getter uses that native field; the accessor result is validated and cached
once per class. Static methods and stored static Fields are rejected. An explicit `options.field`
always wins.
Records are pulled in bounded chunks of `options.batchSize` rows, or 1,024 rows when it is unset.
When `commitRowSize` is set, chunks also end exactly at the next publication boundary and at a
global `maxRowSize` boundary. The first chunk fixes the Arrow JS physical schema and later chunks
use those same column types.

```javascript
const { Field, IOBase, MimeType, fields } = require('yggdryl')

class Trade {
  constructor(row) {
    Object.assign(this, row)
  }

  static get intoStructField() {
    return fields.struct(
      'trade',
      [Field.from('id: int64'), Field.from('venue: utf8')],
      { nullable: false },
    )
  }
}

const handle = IOBase.fromBytes()
handle.mediaType = MimeType.ARROW_STREAM
handle.overwriteRecords(new Trade({ id: 1n, venue: 'XNAS' }))
handle.appendRecords([new Trade({ id: 2n, venue: 'XNYS' })])
const typed = [...handle.readRecords(Trade)]
```

For a synchronous iterable, the native `BatchReader` pulls one copied IPC chunk only when the Rust
write asks for it, so the whole source is never held. With no `commitRowSize`, one logical write
publishes once at end of input. A synchronous Rust write cannot await an async JavaScript iterator,
so an unbounded async write spools the same bounded copied IPC chunks to one private temporary file,
replays that file through one native reader, and removes it on success or failure. This bounds RAM
without claiming zero-copy.

With a positive `commitRowSize`, an async record write does not spool. It alternates one awaited IPC
chunk with one synchronous push into an opaque Rust session. The session owns the global row/byte
limits, fixed leaf/folder/table target, and incomplete cadence; overwrite publishes its first
cadence as overwrite and later cadences as append, while append and merge retain their intent. A
failure after a complete cadence leaves that prefix visible and drops only the incomplete one. A
non-dividing pair such as `batchSize = 1024` and `commitRowSize = 1500` pulls 1024 then 476 records,
so record 1501 is never converted before the 1500-row prefix is published.

`maxRowSize = 0` or `maxByteSize = 0` is a deterministic synchronous exception: append is a no-op,
overwrite publishes an explicitly declared empty `options.field`, and merge with a limit is
rejected. None of these paths inspects the ignored source. Otherwise async record calls return
`Promise<void>`; synchronous record input and all Arrow methods return `void`.

An empty record iterable carries no inferable columns and therefore requires `options.field`.

## Iceberg is a namespace

The table format sits on top of the record encodings in the core, so it is one name here rather than
a handful of top-level classes.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const arrow = require('apache-arrow')
const { Field, fields, iceberg } = require('yggdryl')

const schema = fields.struct('row', [Field.from('id: int64'), Field.from('venue: utf8')], {
  nullable: false,
})

const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'trades')
const table = iceberg.Table.create(root, schema, ['venue'])
table.append(
  new arrow.Table({
    id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
    venue: arrow.vectorFromArray(['XNAS', 'XNYS'], new arrow.Utf8()),
  }),
)

assert.equal(table.currentSnapshot.operation, 'append')
assert.equal(table.dataFiles().length, 2)
assert.equal(table.scan().intoTable().numRows, 2)

fs.rmSync(path.dirname(root), { recursive: true, force: true })
```

`iceberg.Table`, `iceberg.Catalog`, `iceberg.IcebergOptions`,
`iceberg.PartitionSpec`, `iceberg.PartitionField`, `iceberg.Snapshot`,
`iceberg.SnapshotRef`, `iceberg.ManifestFile`, `iceberg.DataFile`,
`iceberg.ScanPlan`, and `iceberg.Compaction` are the
classes; `iceberg.assignFieldIds`, `iceberg.canPromote`,
`iceberg.schemaFromJson`, and `iceberg.schemaToJson` are the functions.
These immutable result values expose `equals`, `compare`, `stableHash`, and
`clone` over their complete Rust-core identity. Snapshot v1 `manifests`, v3 lineage and
encryption key, manifest encryption and partition summaries, and complete
data-file metadata stay available rather than being dropped at the boundary.
`ScanPlan` is a bounded immutable report whose complete
identity, in comparison order, is `(recordCount, filesPlanned, filesSkipped,
manifestsRead, manifestsSkipped)`; physical scan tasks and paths are not part
of that public report. A 64-bit identifier crosses as a `bigint` so a snapshot
id past 2^53 is exact.

## An Iceberg table end to end

A warehouse is one `iceberg.Catalog` over a folder, and a dotted name is all a writer needs: the
first append creates the table from the rows' own schema. Every rows argument here is widened by
`BatchReader.from` exactly as it is on `IOBase`, so an Apache Arrow JS table appends directly. A
column change is a chain recorded on `updateSchema()` and committed once, `compact()` rewrites
undersized files as one `replace` snapshot and reports what it rewrote, and `scanAt` reads any
retained snapshot - named by a `bigint` or an exact `number` - as a complete table.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const arrow = require('apache-arrow')
const { iceberg } = require('yggdryl')

const warehouse = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-doc-'))
const catalog = new iceberg.Catalog(warehouse)

// Rows and a dotted name are enough: the first append creates the table.
const rows = (ids, venues) =>
  new arrow.Table({
    id: arrow.vectorFromArray(ids, new arrow.Int64()),
    venue: arrow.vectorFromArray(venues, new arrow.Utf8()),
  })
const table = catalog.append('nyc.trades', rows([1n, 2n], ['XNAS', 'XNYS']))
const past = table.currentSnapshot.snapshotId
table.append(rows([3n], ['XASE']))
assert.deepEqual(catalog.namespace('nyc').tables.names(), ['trades'])
assert.equal(table.scan().intoTable().numRows, 3)

// A column change is a chain recorded on the update, committed once.
table.updateSchema().addColumn('', 'price: float64').commit()
assert.equal(table.scan().intoTable().getChild('price').get(0), null)

// Undersized files rewrite as one replace commit that reports itself.
const compaction = table.compact()
assert.equal(compaction.filesBefore, 2)
assert.equal(compaction.filesAfter, 1)
assert.equal(table.scan().intoTable().numRows, 3)

// And nothing rewrote history: the first snapshot reads as it was written.
assert.deepEqual(
  table.scanAt(past).intoTable().getChild('id').toArray(),
  BigInt64Array.from([1n, 2n]),
)

fs.rmSync(warehouse, { recursive: true, force: true })
```

The walk is the same in every language: the [iceberg](../iceberg.md) page shows each of these
steps beside its Rust and Python form.

## Errors

A native error crosses unchanged and arrives as a `TypeError` or `RangeError` carrying the message
the Rust error produced, including its path or byte offset.

```javascript
const { DataType } = require('yggdryl')
const assert = require('node:assert/strict')

assert.throws(() => DataType.from('decimal(0,0)'), /precision/)
```

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[JavaScript](../notebooks/javascript/extensions_javascript.ipynb){ download }.

<!-- /notebooks -->
