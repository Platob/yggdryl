# JavaScript

What the Node-API package adds on top of the [core](../index.md), and how
values cross the JavaScript boundary.

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

Every constructor accepts the obvious JavaScript spelling and converts once, in
Rust.

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

There is no JavaScript-side parser: `DataType.from` and
`DataType.fromRegex(pattern, autotype)` call the native constructor. Variadic
`Uri.joinPath` replaces the `/` operator idiom, normalizing through the same
core `joinpath`.

## One native field from a class or value

`intoField(value, name?)` and the class-level `intoStructField` static getter
follow the canonical [field](../types/field.md) conversion contract. The result
is validated as a non-null Struct field and memoized per class.

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

A native `Field` or a field expression is accepted too.

## Scalars cross as their natural shape

A JavaScript value becomes the nearest native scalar, and comes back as the
nearest JavaScript value to that.

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

Reconstructing a lost shape takes your own code.

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

No name in a document makes this binding look up a class or run a constructor.
`Date`, `Buffer`, and `Map` are read off the intrinsic prototypes.

## Scalar families

`Scalar.float(value, width = 64)` selects 16, 32, or 64 bits, and
`decimal(coefficient, scale = 0)` the narrowest exact decimal. Temporal
factories take `(count, unit, timezone)`, `date` defaults to days, and an
omitted timezone is `NAIVE`.

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

`kind`, `count`, `unit`, `zone`, `unscaled`, and `scale` expose the payload.
`asBytes`/`asUtf8` borrow content, and `asJsonBytes`/`asJsonUtf8` use the core
natural JSON writer.

## Native value protocols and checked arithmetic

Immutable native values expose `equals`, `compare`, `stableHash`, and `clone`
wherever the core identity is complete. JavaScript cannot overload arithmetic,
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

`Expression` exposes the same names as lazy tree builders. A string operand is
parsed as expression text, and any other JavaScript value becomes a literal.

## fromJs and asJs

`Scalar.fromJs` and `Scalar.prototype.asJs` are the conversion pair every
`loads` and `dumps` crosses.

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
children, while `set` and `remove` rebuild without mutating the source.

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
are validated accessors, not map keys. Each well-known protocol is a getter over a live `Map`
view of the same field.

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

`field.protocol(name)` takes a runtime scheme, and there is no `https` getter,
because HTTPS shares the canonical `http:` namespace. A schema also says which
columns a path spells out.

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

Arrow JS ships no filesystem, so `IOBase.fromFs(handler, path)` turns a plain
object into an ordinary handle.

- Arrow's own `FileSystem` calls in camelCase: `fileInfo`, `list`, `readRange`,
  `writeFull`, `createDir`, `deleteFile`, and `typeName`.
- Sizes cross as `bigint`; an exact `number` is accepted too.
- `using` binds to `open`/`close`, which publishes a staged whole value.

[Filesystems](../holder/backends/filesystems.md) shows a complete handler.

## Bytes and ranges

`readRangeBytes` and `appendBytes` are the core's `read_range_bytes` and
`append_bytes`, camelCased. `readRange` chooses the answer's type from its
options, and `append` chooses how to read its byte source.

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

`append` takes a typed array, `DataView`, `ArrayBuffer`, or UTF-8 string, and
answers the byte offset it reached. [Bytes](../holder/iobase/bytes.md) states
what the primitives do.

## Arrow

Apache Arrow JS values cross the boundary as copied IPC, never zero-copy.

```javascript
const { DataType } = require('yggdryl')
const assert = require('node:assert/strict')

const scalar = DataType.from('int64').defaultArrowScalar()
assert.equal(String(scalar), '0')
```

`defaultJSValue`, `defaultJSHint`, and `defaultArrowScalar` project the native
default planner; JavaScript caches identity but decides nothing.

## Records: explicit intent and representation

`BatchReader` is the primitive record shape: `readArrowReader` returns one and
each `overwriteArrowReader`/`appendArrowReader`/`mergeArrowReader` consumes one.
`BatchReader.from` accepts another reader, an Arrow JS `Table` or `RecordBatch`,
an array of batches, or IPC bytes.

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

Each batch crosses as its own IPC stream, so its schema travels with it.
`intoIpc` drains the reader into one stream, `intoTable` into one Arrow JS
table.

`recordOptions()` derives the encoding from the handle's media type, so no call
names it. `RecordOptions` compares, hashes, and clones over the complete core
value, every encoding-specific setting included.

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

| Input | Overwrite | Append | Key-matched merge |
| --- | --- | --- | --- |
| native `BatchReader` | `overwriteArrowReader` | `appendArrowReader` | `mergeArrowReader` |
| Arrow JS `Table` | `overwriteArrowTable` | `appendArrowTable` | `mergeArrowTable` |
| Arrow JS `RecordBatch` | `overwriteArrowBatch` | `appendArrowBatch` | `mergeArrowBatch` |
| plain objects or field-class instances | `overwriteRecords` | `appendRecords` | `mergeRecords` |

Configured mode reaches one dispatcher per shape.

```text
writeArrowReader(reader, mode, options?)
writeArrowTable(table, mode, options?)
writeArrowBatch(batch, mode, options?)
writeRecords(records, mode, options?)
```

`mode` is `'overwrite'`, `'append'`, or `'merge'`, validated before any reader,
exporter, or iterator is touched. Table and batch methods validate their named
shape, infer a native `Field`, and copy one IPC stream.

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

Plain objects infer one struct field through Arrow JS and a class instance uses
its static `intoStructField` getter, but an explicit `options.field` wins.
Records are pulled in chunks of
`options.batchRowSize` rows, 1,024 when unset.

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

An unbounded async write spools bounded IPC chunks to one private temporary
file, because a synchronous Rust write cannot await. A positive `commitRowSize`
does not spool: overwrite publishes its first cadence as overwrite, later ones
as append.

## Iceberg is a namespace

The table format is one namespace rather than several top-level classes.

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

`Table`, `Catalog`, `IcebergOptions`, `PartitionSpec`, `PartitionField`,
`Snapshot`, `SnapshotRef`, `ManifestFile`, `DataFile`, `ScanPlan`, and
`Compaction` are the classes; `assignFieldIds`, `canPromote`, `schemaFromJson`,
and `schemaIntoJson` are the functions.

## An Iceberg table end to end

A warehouse is one `iceberg.Catalog` over a folder, and a dotted name is all a
writer needs. Every rows argument is widened by `BatchReader.from`, so an Arrow
JS table appends directly.

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

The [iceberg](../media/iceberg/index.md) pages show each step in Rust and
[Python](python.md).

## Digests

`xxhash` carries the four one-shot functions, the four resumable states, and
`Digest`; `IOBase.readDigest` and `Scalar.digest` reach the same native path.
XXH32 answers a `number`, the wider algorithms `bigint`.

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

Every immutable wrapper here follows the same convention: `equals`, `compare`,
`stableHash`, `clone`, `toString`, and `toJSON`. A `Buffer` or `Uint8Array` is
hashed in place, an `ArrayBuffer` is narrowed to a `Buffer` window, and a
`string` is encoded as UTF-8.

## FIX is a namespace

`fix.FixRegistry`, `fix.FixMsg`, `fix.globalRegistry()`,
`fix.installGlobalRegistry()`, `fix.STANDARD_BRANCH` (`'standard'`), and
`fix.STANDARD_TAG_LIMIT` (`5000`) are the whole surface. The `fix:` vocabulary
is six accessor pairs on the `field.fix` view: `branch`, `id`, `tag`, `tags`,
`aliases`, and `description`.

| Crossing | Rule |
| --- | --- |
| tag key | a `number`, coerced once and checked exactly |
| name or path key | a `string`, standard branch; a colon-bearing string is a name |
| branch, identifier | `string`, parsed by the core `FixBranch` and `FixId` |
| `fieldByName`, `fieldByPath` | take the branch as leading argument |
| `fieldByTag` | means the standard branch exactly |
| `field.fix.id` | `'branch:tag'`, `null` exactly when `fix:tag` is absent; assigning one moves both halves |
| `field.fix.branch` | `'standard'` when the key is absent; assigning `'standard'` removes it |
| `message.at`, `message.byId` | the failing halves; `value` holds the whole message value |
| `fromHandle`, `writeInto` | an `IOBase`, a `Url`, or the string naming one |
| iteration | registry branch-major then by tag, message in the root's declared order |

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

`FixMsg`'s constructor is the one widening gate: the core alone types, orders,
and validates a plain object. Resolution and merging are the core's, on the
[fix](../fix/index.md) pages.

## Edges

- Any native refusal -> `TypeError` or `RangeError` with the Rust message, path
  or byte offset included.
- Streaming `reader`/`writer`, the handle wrappers, `Hashed<H>`, and the
  per-protocol view types -> Rust-only.
- Rust spellings -> `as_iceberg()`/`as_iceberg_mut()`, `contentType` as
  `as_http().content_type()`, `arrow` as `as_arrow_properties`, and
  `fieldProperties` as `as_field_properties`.
- `DataType.fromRegex` -> named-capture inference, decided by the core.
- `gzip`, `zlib`, `zstd` -> `loads`/`dumps` over `Buffer`, plus
  `loadsRaw`/`dumpsRaw` on `zlib`.
- `TextOptions.withRownum` given a `number`, or a `bigint` wider than 128 bits
  -> refused, never narrowed or rounded.
- A handler-backed handle in a `Worker` -> refused by name; handlers run
  synchronously on the thread that supplied them.
- `readRange` with an unknown option, a non-boolean `text`, or an undecodable
  range -> refused.
- Any class member but the static `intoStructField` getter -> rejected.
- `time` and `duration` -> `NAIVE` only; `datetime` also takes a name or `Timezone`.
- Date and time widths -> follow the unit, duration follows the count, and Arrow
  datetime is always 64-bit.
- Exact `kind` values -> survive field-directed transport.
- Schemaless text -> ISO strings; `loads(..., { field })` restores the exact type.
- An invalid operand kind -> `TypeError` `ERR_YGGDRYL_INVALID_ARITHMETIC`;
  overflow, division by zero, and inexact decimal division -> `RangeError`
  `ERR_YGGDRYL_ARITHMETIC_OVERFLOW`, `ERR_YGGDRYL_DIVISION_BY_ZERO`,
  `ERR_YGGDRYL_INEXACT_ARITHMETIC`.
- `Scalar`, `Expression`, `Statement`, `avro.Schema`, and the `iceberg` result
  values -> complete identity; readers, iterators, and handles -> none.
- `compare` -> the core total order; equal values share one deterministic
  `stableHash()` `bigint`, and `===` stays reference identity.
- `Scalar` equality, order, and hashing -> normalize equivalent decimal and
  temporal resolutions.
- A `Scalar` binary operand -> a native `Scalar`, or one JavaScript value read
  through `Scalar.fromJs`; the answer is a native `Scalar`.
- Numeric arithmetic -> preserves widths and promotes only as the core defines;
  exact decimal division takes the smallest terminating scale.
- Text and containers -> never concatenated or coerced; datetime and duration
  pairs use the same checked path.
- `avro.Schema` identity -> keeps logical types, defaults, aliases, and
  extension attributes; `fingerprint()` alone follows Parsing Canonical Form.
- `iceberg.IcebergOptions` -> compares over its current explicit configuration,
  so a mutated clone stops comparing equal.
- `{ maxDepth }` -> inclusive 1 to 48; codec loads add byte, node, and document
  limits.
- `rowSize`, `columnSize` -> the whole logical media, never the last projection,
  filter, or row limit read through it.
- `rowSize`, `columnSize` -> lazy, cached while `open()` holds the metadata,
  invalidated by a write, recomputed after `close()`, saturating at
  `Number.MAX_SAFE_INTEGER`.
- A setting one encoding has -> `null` on the others; `isIo()` without either
  surface -> `false`.
- `mergeByNames` -> rejected by overwrite and append, required non-empty by
  merge, before anything is consumed.
- A reader method -> only a native `BatchReader`; `BatchReader.from(value)`
  converts anything else.
- `RecordOptions` identity for Text -> declared regex source and extractor
  settings participate, compiled caches and derived fields do not.
- A set `commitRowSize` -> chunks end at each publication and `maxRowSize`
  boundary; with none, one write publishes at end of input.
- The first record chunk -> fixes the Arrow JS physical schema, and later chunks
  reuse those column types.
- `batchRowSize = 1024` with `commitRowSize = 1500` -> pulls 1024 then 476, so
  record 1501 waits for the 1500-row prefix.
- The async spool file -> removed on success or failure; append and merge keep
  their intent across cadences.
- `maxRowSize = 0` or `maxByteSize = 0` -> append no-ops, overwrite publishes
  the declared empty `options.field`, merge is rejected; neither reads the source.
- An empty record iterable -> requires `options.field`.
- A failure after a complete cadence -> that prefix stays visible, only the
  incomplete one drops.
- Async record calls -> `Promise<void>`; sync records and Arrow -> `void`.
- Snapshot v1 `manifests`, v3 lineage and encryption key, manifest encryption
  and partition summaries, and full data-file metadata -> kept at the boundary.
- `iceberg.ScanPlan` -> `(recordCount, filesPlanned, filesSkipped,
  manifestsRead, manifestsSkipped)` in comparison order; tasks and paths stay
  private.
- A snapshot id past 2^53 -> exact, since identifiers cross as `bigint`;
  `scanAt` takes a `bigint` or an exact `number`.
- A fractional tag or one outside `i32` -> throws; a `bigint`, object, or `null`
  key -> `TypeError`.
- Registry mutation while a `FixMsg`, the process default, or a live `keys()`
  walk holds it -> throws; `registry.clone()` is the mutable deep copy.
- A `keys()` walk -> stops sharing when drained, or when a `for...of` `break`
  returns it.
- FIX absence -> the native refusal, or `null` from the `get`-prefixed twins,
  for a key that parses.
- A missing FIX folder -> the empty registry; a retired `records/` folder -> throws.
- A registry write -> creates the folder and its parents under
  `primitive/<branch>/` and `nested/<branch>/`.
- `message.getById`/`byId` -> name one dictionary exactly and do not tier.

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

    `npm test` runs the suite and then `tsc --noEmit` over the shipped `.d.ts`
    declarations.

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
