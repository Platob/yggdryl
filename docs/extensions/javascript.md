# JavaScript

The Node-API package: what it adds on top of the [core](../index.md), and how each
layer's values cross the JavaScript boundary.

## Contract

| Name | Documented in |
| --- | --- |
| `DataType` | [datatype](../types/datatype.md) |
| `Field`, `fields` | [field](../types/field.md) |
| `Expression`, `Bound`, `Statement`, `BoundStatement` | [expression](../expression/index.md) |
| `Uri`, `Url`, `Urn` | [uri](../uri/index.md) |
| `IOBase` | [holder](../holder/index.md) |
| `BatchReader`, `RecordOptions` | [records](../holder/iobase/records.md), [options](../media/options.md) |
| `iceberg` | [iceberg](../media/iceberg/index.md) |
| `fix` | [fix](../fix/index.md) |
| `MimeType`, `MediaType`, `Timezone` | [enums](../types/scalar.md) |
| `codec`, `json`, `toml`, `yaml`, `Scalar` | [text](../text/index.md) |
| `avro` | [Avro](../media/avro.md) |
| `gzip`, `zlib`, `zstd` | [coding](../coding/index.md) |
| `xxhash`, `Digest` | [xxhash](../xxhash/index.md) |

## Use

Every constructor accepts the obvious JavaScript spelling of its argument and
converts once, in Rust. Prefer the generic `from` entry points.

```javascript
const { DataType, Field, MediaType, MimeType, Uri, Url } = require('yggdryl')
const assert = require('node:assert/strict')

// A datatype expression is a datatype.
assert.equal(String(new Field('id', 'int64', false).dtype), 'int64')
assert.equal(DataType.from('list<int32>').kind, 'nested')

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

There is no JavaScript-side parser: `DataType.from`, `DataType.fromRegex`, and
their siblings call the matching native constructor. Variadic `Uri.joinPath`
replaces the `/` operator idiom, normalizing through the same core `joinpath`.

## One native field from a class or value

`intoField(value, name?)` and the class-level `intoStructField` static getter
follow the canonical [field](../types/field.md) conversion contract. The getter
result is validated as a non-null Struct field and memoized per class.

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

The converter also accepts a native `Field` or a field expression.

## Scalars cross as their natural shape

A JavaScript value becomes the nearest native scalar, and comes back as the
nearest JavaScript value to that. No class name travels beside the data.

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

The losses are deliberate, and reconstructing any of them needs no cooperation
from the codec.

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

No name in a document can make this binding look one up or run a constructor.
`Date`, `Buffer`, and `Map` are read off the intrinsic prototypes, so no
replaceable method decides them.

## Scalar families

`Scalar.float(value, width = 64)` selects 16, 32, or 64 bits, and
`decimal(coefficient, scale = 0)` selects the narrowest exact decimal. Temporal
factories take `(count, unit, timezone)`, `date` defaults to days, and an
omitted timezone becomes `NAIVE`.

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
payload. `asBytes`/`asUtf8` borrow scalar content, while `asJsonBytes` and
`asJsonUtf8` use the core natural JSON writer.

## Native value protocols and checked arithmetic

Immutable native values expose `equals`, `compare`, `stableHash`, and `clone`
where the core identity is complete, including `Scalar`, `Expression`,
`Statement`, `avro.Schema`, `iceberg.PartitionSpec`, `iceberg.DataFile`, and
`iceberg.ScanPlan`. JavaScript cannot overload equality, hashing, or arithmetic,
so `Scalar` exposes checked `add`, `subtract`, `multiply`, `divide`,
`remainder`, `negate`, and `absolute`.

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

`Expression` exposes the same arithmetic names as lazy tree builders over a
native operand, parsed expression text, or an inferred literal. `Scalar`
equality, order, and hashing normalize equivalent decimal and temporal
resolutions.

## fromJs and asJs

`Scalar.fromJs` and `Scalar.prototype.asJs` are the conversion pair every
`loads` and `dumps` crosses. Calling them directly shows what a value becomes
before any format is involved.

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

`length`, iteration, `at`, `get`, `has`, and `path` return exact native
children, while `set` and `remove` rebuild without mutating the source. Both
accept nullable `{ maxDepth }` for language traversal.

## Field metadata is a Map

`Field` implements the `Map` protocol over its metadata, in the native ordering.

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

Typed identifiers and typed HTTP values (`dictionaryId`, `contentType`, `etag`)
are validated accessors rather than map keys. Each well-known protocol is a
getter whose properties are a live `Map` view of the same field.

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

`field.protocol(name)` takes a scheme known only at runtime. There is no `https`
getter, because HTTPS shares the canonical `http:` namespace.

A schema also says which of its columns a path spells out, which a partitioned
write and an Iceberg spec both read.

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
assert.equal(schema.dtype.getFieldByPath('year').isPartition, true)
assert.equal(schema.withoutPartitionFields().dtype.length, 1)
```

## A filesystem is whatever answers seven calls

Arrow JS ships no filesystem, so JavaScript supplies the vtable itself.
`IOBase.fromFs(handler, path)` turns a plain object into an ordinary handle.

- Methods are Arrow's own `FileSystem` calls in camelCase: `fileInfo`, `list`,
  `readRange`, `writeFull`, `createDir`, `deleteFile`, plus a `typeName`.
- Sizes cross as `bigint`, because a 64-bit length is what an object store
  reports; an exact `number` is accepted too.
- `using` binds to `open`/`close`, which publishes a staged whole value.

[Filesystems](../holder/backends/filesystems.md) documents the backend and shows
a complete handler.

## Bytes and ranges

`readRangeBytes` and `appendBytes` are the core's `read_range_bytes` and
`append_bytes`, camelCased and nothing more. Over each sits one inferring entry
point: `readRange` chooses the answer's type from its options, and `append`
chooses how to read its byte source.

```javascript
const assert = require('node:assert/strict')
const { IOBase } = require('yggdryl')

const handle = IOBase.fromBytes(Buffer.from('symbol,price\n'))

// `{ text: true }` selects the answer's type; omitting it answers a Buffer.
assert.deepEqual(handle.readRange(0, 6), Buffer.from('symbol'))
assert.equal(handle.readRange(0, 6, { text: true }), 'symbol')

// `append` takes a string, any view, and an ArrayBuffer as well as a Buffer.
assert.equal(handle.append('AAPL,1\n'), 13)
assert.equal(handle.append(new Uint8Array([77, 83, 70, 84, 10])), 20)
assert.equal(handle.append(new DataView(Uint8Array.from([78, 86, 68, 65, 10]).buffer)), 25)
assert.equal(handle.append(Uint8Array.from([73, 78, 84, 67, 10]).buffer), 30)
assert.equal(handle.readRange(13, 7, { text: true }), 'AAPL,1\n')

// A range it cannot decode is refused, not silently substituted.
assert.throws(() => IOBase.fromBytes(Buffer.from([0xff])).readRange(0, 1, { text: true }))
```

`append` reads any typed array or `DataView` over its own window, an
`ArrayBuffer`, and a string encoded as UTF-8, and returns the byte offset it
landed at. [Bytes](../holder/iobase/bytes.md) states what the primitives do.

## Arrow

Apache Arrow JS values cross the boundary as copied IPC. This is not a zero-copy
bridge.

```javascript
const { DataType } = require('yggdryl')
const assert = require('node:assert/strict')

const scalar = DataType.from('int64').defaultArrowScalar()
assert.equal(String(scalar), '0')
```

`defaultJSValue`, `defaultJSHint`, and `defaultArrowScalar` are schema-directed
projections of the native default planner. The JavaScript layer caches identity
but never decides what a default is.

## Records: explicit intent and representation

`BatchReader` is the primitive record shape: `readArrowReader` returns one, and
each `overwriteArrowReader`/`appendArrowReader`/`mergeArrowReader` call consumes
one. `BatchReader.from` accepts another reader, an Arrow JS `Table` or
`RecordBatch`, an array of batches, or Arrow IPC bytes.

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

Each batch crosses as its own self-contained Arrow IPC stream, so its schema
travels with it and Arrow JS needs no separate handshake. `intoIpc` drains the
reader into one stream and `intoTable` into one Arrow JS table.

`recordOptions()` derives the encoding from the handle's media type, so no call
names it. `RecordOptions` exposes `equals`, `compare`, `stableHash`, and `clone`
over the complete core value, every encoding-specific setting included.

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

The explicit method name carries both facts.

| --- | --- | --- | --- |
| native `BatchReader` | `overwriteArrowReader` | `appendArrowReader` | `mergeArrowReader` |
| Arrow JS `Table` | `overwriteArrowTable` | `appendArrowTable` | `mergeArrowTable` |
| Arrow JS `RecordBatch` | `overwriteArrowBatch` | `appendArrowBatch` | `mergeArrowBatch` |
| plain objects or field-class instances | `overwriteRecords` | `appendRecords` | `mergeRecords` |

When mode comes from configuration, each shape also has one dispatcher, taking
input, required mode, then the one optional settings value.

```text
writeArrowReader(reader, mode, options?)
writeArrowTable(table, mode, options?)
writeArrowBatch(batch, mode, options?)
writeRecords(records, mode, options?)
```

`mode` is `'overwrite'`, `'append'`, or `'merge'`, validated before any reader,
exporter, or iterator is touched. Table and batch methods validate their named
shape, infer a native `Field`, and copy one IPC stream at the boundary.

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

### Records and field classes

Plain objects infer one struct field through Arrow JS, while a class instance
uses its static `intoStructField` getter, validated and cached once per class.
Records are pulled in bounded chunks of `options.batchRowSize` rows, or 1,024
rows when it is unset.

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

A synchronous Rust write cannot await an async JavaScript iterator, so an
unbounded async write spools bounded IPC chunks to one private temporary file.
With a positive `commitRowSize` it does not spool: overwrite publishes its first
cadence as overwrite and later cadences as append.

## Iceberg is a namespace

The table format sits on top of the record encodings in the core, so it is one
name here rather than a handful of top-level classes.

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

`iceberg.Table`, `Catalog`, `IcebergOptions`, `PartitionSpec`, `PartitionField`,
`Snapshot`, `SnapshotRef`, `ManifestFile`, `DataFile`, `ScanPlan`, and
`Compaction` are the classes. `assignFieldIds`, `canPromote`, `schemaFromJson`,
and `schemaIntoJson` are the functions.

## An Iceberg table end to end

A warehouse is one `iceberg.Catalog` over a folder, and a dotted name is all a
writer needs. Every rows argument is widened by `BatchReader.from`, so an Apache
Arrow JS table appends directly.

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

The walk is the same in every language: the
[iceberg](../media/iceberg/index.md) pages show each step beside its Rust and
[Python](python.md) form.

## Digests

`xxhash` carries the four one-shot functions, the four resumable states, and
`Digest`; `IOBase.readDigest` and `Scalar.digest` reach the same native path.
XXH32 answers a `number`, and the wider algorithms answer `bigint`.

```javascript
const assert = require('node:assert/strict')
const { Scalar, xxhash } = require('yggdryl')

const payload = Buffer.from('abc')
assert.equal(xxhash.xxh32(payload), 0x32d153ff)
assert.equal(xxhash.xxh3(payload), 0x78af5f94892f3950n)
assert.equal(xxhash.xxh3('abc'), xxhash.xxh3(new Uint8Array(payload)))

const digest = xxhash.digest(payload, 'xxh3-64')
assert.equal(digest.toString(), 'xxh3-64:78af5f94892f3950')
assert.ok(xxhash.Digest.from(digest.toString()).equals(digest))
assert.equal(Scalar.fromJs('AAPL').digest().value(), Scalar.fromJs('AAPL').stableHash())
```

`Digest` follows the convention every immutable wrapper here follows: `equals`,
`compare`, `stableHash`, `clone`, `toString`, and `toJSON`. A `Buffer` or
`Uint8Array` is hashed in place, an `ArrayBuffer` is narrowed to a `Buffer`
window, and a `string` is encoded as UTF-8.

## FIX is a namespace

`fix.FixRegistry`, `fix.FixMsg`, `fix.globalRegistry()`,
`fix.installGlobalRegistry()`, `fix.STANDARD_BRANCH` (`'standard'`), and
`fix.STANDARD_TAG_LIMIT` (`5000`) are the whole surface. The `fix:` vocabulary
is six accessor pairs on the `field.fix` view: `branch`, `id`, `tag`, `tags`,
`aliases`, `description`.

| Crossing | Rule |
| --- | --- |
| tag key | a `number`, coerced once and checked exactly |
| name or dotted-path key | a `string`, in the standard branch; a colon-bearing string is a name |
| branch, identifier | `string`, parsed once through `FixBranch::from_str` and `FixId::from_str` |
| `getFieldByName`/`fieldByName`, `getFieldByPath`/`fieldByPath` | take the branch as their leading argument |
| `getFieldByTag`/`fieldByTag` | mean the standard branch exactly |
| `field.fix.id` | the `'branch:tag'` text, `null` exactly when `fix:tag` is absent |
| `field.fix.branch` | `'standard'` when the key is absent; assigning `'standard'` removes it |
| `FixRegistry.fromHandle`, `registry.writeInto` | an `IOBase`, a `Url`, or the string naming one |
| iteration | registry by ascending canonical identifier, message in the root's declared order |
| `message.at`, `message.byId` | the failing halves; `value` holds the whole message value |

```javascript
const assert = require('node:assert/strict')
const path = require('node:path')
const { Field, IOBase, Url, fields, fix } = require('yggdryl')

const seed = path.resolve('config/fix')

// One folder, named however JavaScript names one - the coercion `Catalog` uses.
const url = Url.fromPath(seed)
for (const location of [seed, url.toString(), url, new IOBase(seed)]) {
  assert.equal(fix.FixRegistry.fromHandle(location).size, 34)
}
const registry = fix.FixRegistry.fromHandle(seed)

// A key is a number tag or a string name, and a tag that would not fit i32 is
// refused rather than narrowed into a different one.
assert.ok(registry.get(55).equals(registry.get('symbol')))
assert.throws(() => registry.getFieldByTag(2 ** 31), /tag must be a signed 32-bit integer/)
assert.throws(() => registry.get(55n), {
  name: 'TypeError',
  message: 'key must be a number tag or a string name, got BigInt',
})

// A branch and an identifier cross as text: the branch leads a name or path
// lookup, and a malformed one throws rather than missing.
assert.equal(fix.STANDARD_BRANCH, 'standard')
assert.equal(fix.STANDARD_TAG_LIMIT, 5000)
assert.equal(registry.fieldByName(fix.STANDARD_BRANCH, 'ticker').name, 'Symbol')
assert.equal(registry.fieldByPath(fix.STANDARD_BRANCH, 'NoPartyIDs.PartyID').fix.tag, 448)
assert.equal(registry.fieldById('standard:55').fix.id, 'standard:55')
assert.throws(() => registry.fieldByName('2cme', 'Symbol'), /fix branch/)
assert.throws(() => registry.fieldById('55'), /fix identifier/)
assert.throws(() => registry.fieldById(55), /into rust type `String`/)

// Absence throws with the native message; the `get` half answers null.
assert.throws(
  () => registry.fieldByName(fix.STANDARD_BRANCH, 'Nope'),
  /expected a fix field at "name/,
)
assert.equal(registry.getFieldByName(fix.STANDARD_BRANCH, 'Nope'), null)
assert.throws(() => registry.insert(Field.from('Untagged: utf8')), /fix:tag/)

// The typed vocabulary is answered by the fix view alone, and a tag the FIX
// specification assigns cannot move to another dictionary.
const symbol = registry.fieldByTag(55)
assert.equal(symbol.fix.tag, 55)
assert.equal(symbol.fix.id, 'standard:55')
assert.throws(() => symbol.iceberg.tag, { name: 'TypeError', message: /iceberg/ })
const vendor = Field.from('TradeID: utf8')
vendor.fix.id = 'CME:5001'
assert.equal(vendor.fix.id, 'cme:5001')
assert.equal(vendor.fix.branch, 'cme')
assert.throws(() => {
  vendor.fix.tag = 35
}, /fix:branch/)
assert.equal(vendor.fix.id, 'cme:5001')

// A message shares the dictionary it resolved against, so mutating it refuses.
const root = fields.struct('row', [symbol], { nullable: false })
const message = new fix.FixMsg(root, { Symbol: 'AAPL' }, registry)
assert.throws(() => registry.remove(55), /shared with a message/)
assert.equal(registry.clone().remove(55).name, 'Symbol')

// A vendor field leaves by its identifier: `remove` reads a string as a
// standard-branch name.
const venue = fix.FixRegistry.fromFields([vendor])
assert.equal(venue.remove('TradeID'), null)
assert.equal(venue.removeById('cme:5001').name, 'TradeID')
assert.equal(venue.size, 0)

// Both collections are lazy native iterators the loader gives the protocol.
assert.equal([...registry].length, 34)
assert.deepEqual([...message].map(([name]) => name), ['Symbol'])
assert.equal(message.at('ticker').asJs(), 'AAPL')
assert.equal(message.branch, fix.STANDARD_BRANCH)
assert.equal(message.byId('standard:55').asJs(), 'AAPL')
assert.equal(message.getById('cme:5001'), null)
```

`FixMsg`'s constructor is the one widening gate: a plain object is read through
`Scalar.fromJs`, and the core alone types, orders, and validates it. Resolution,
folding, merging, and sharding are the core's, on the [fix](../fix/index.md) pages.

## Edges

- A native refusal -> a `TypeError` or `RangeError` carrying the Rust message,
  including its path or byte offset.
- Coding and xxHash streaming `reader`/`writer`, the transparent handle
  wrappers, and `Hashed<H>` -> Rust-only, built on Rust `Read`/`Write`.
- `gzip`, `zlib`, `zstd` -> `loads`/`dumps` over `Buffer`, plus `loadsRaw`/`dumpsRaw`
  on `zlib`, reading and writing exactly what `node:zlib` does.
- A named handle -> applies the coding its name declares; `IOBase.codec` asks
  which one that is.
- `TextOptions.withRownum` given a `number` -> rejected, never silently
  narrowed; it is `bigint | null`.
- A handler-backed handle in a `Worker` -> refused by name, because the handler
  is called synchronously on the thread that supplied it.
- `readRange` with an unknown option or a non-boolean `text` -> rejected;
  `length` is checked as strictly as `offset` rather than rounded.
- A range that cannot decode as text -> refused, not silently substituted.
- A class member form other than the static `intoStructField` getter -> rejected;
  it must resolve to a native Struct field whose root is non-null.
- A `bigint` wider than 128 bits -> refused rather than rounded.
- `time` and `duration` -> accept only `NAIVE`, while `datetime` also accepts a
  timezone name or a `Timezone`.
- Schemaless text -> natural ISO strings; `loads(..., { field })` restores the
  declared exact type.
- An invalid operand kind -> `TypeError` with `ERR_YGGDRYL_INVALID_ARITHMETIC`.
- Overflow, division by zero, non-terminating exact decimal division ->
  `RangeError` with `ERR_YGGDRYL_ARITHMETIC_OVERFLOW`,
  `ERR_YGGDRYL_DIVISION_BY_ZERO`, and `ERR_YGGDRYL_INEXACT_ARITHMETIC`.
- `{ maxDepth }` -> the inclusive range 1 to 48; input-byte, node, and document
  limits apply to codec loads, where a text parser runs.
- Rust per-protocol view types (`HttpField`, `IcebergField`, and the sixteen
  others) -> no JavaScript counterpart; `field.iceberg` answers the generic `Map`.
- `rowSize`, `columnSize` -> lazy, cached while `open()` holds the media
  metadata, invalidated by a write, recomputed after `close()`, and saturating
  at `Number.MAX_SAFE_INTEGER`.
- `isIo()` on a container exposing neither surface -> `false`.
- A setting one encoding has -> `null` on the others, never invented.
- `mergeByNames` -> rejected by overwrite and append, required non-empty by
  merge, before any reader, table, batch, or iterable is touched.
- `maxRowSize = 0` or `maxByteSize = 0` -> a deterministic synchronous
  exception: append is a no-op, overwrite publishes the declared empty
  `options.field`, and merge with a limit is rejected.
- An empty record iterable -> requires `options.field`, having no inferable columns.
- A failure after a complete commit cadence -> that prefix stays visible and
  only the incomplete one is dropped.
- Async record calls -> `Promise<void>`; synchronous record input and every
  Arrow method -> `void`.
- A snapshot id past 2^53 -> exact, because 64-bit identifiers cross as `bigint`.
- A tag that is fractional or outside `i32` -> throws rather than narrowing into
  a different tag; a `bigint`, object, or `null` key -> `TypeError`.
- Registry mutation while a `FixMsg`, the process default, or an unfinished
  `keys()` walk holds it -> throws; `registry.clone()` is the mutable deep copy.
- FIX absence -> the native refusal from `fieldById` and its siblings, `null`
  from the `get`-prefixed twins, but only for a key that parses.
- A FIX folder that is not there -> loads as the empty registry and creates
  nothing; a root still holding the retired `records/` folder throws.

```javascript
const { DataType } = require('yggdryl')
const assert = require('node:assert/strict')

assert.throws(() => DataType.from('decimal(0,0)'), /precision/)
```

## Commands

=== "JavaScript"

    ```bash
    npm install --prefix node
    npm run --prefix node build:debug
    npm test --prefix node
    node --test "node/tests/types/*.test.js" node/tests/enums.test.js
    node --test "node/tests/holder/*.test.js"
    node --test "node/tests/media/*.test.js" node/tests/entrypoints.test.js
    node --test "node/tests/text/*.test.js"
    node --test "node/tests/uri/*.test.js"
    node --test "node/tests/expression/*.test.js"
    node --test "node/tests/xxhash/*.test.js"
    node --test "node/tests/fix/*.test.js"
    npm run --prefix node typecheck
    python scripts/check_docs_examples.py --lang javascript
    ```

    `npm test` runs the suite and then `tsc --noEmit`, so the shipped `.d.ts`
    declarations are checked against the tests that use them.

    ```bash
    npm run --prefix node bench:types
    npm run --prefix node bench:types:defaults
    npm run --prefix node bench:holder
    npm run --prefix node bench:holder:io
    npm run --prefix node bench:coding
    npm run --prefix node bench:media
    npm run --prefix node bench:media:text -- --records 5000 --iterations 3
    npm run --prefix node bench:text
    npm run --prefix node bench:xxhash
    npm run --prefix node bench:fix
    ```

    Every benchmark reads its iteration count from `YGGDRYL_BENCH_ITERATIONS`.
