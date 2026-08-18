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
| `Uri`, `Url`, `Urn` | [uri](../uri.md) |
| `IOBase` | [io](../io.md) |
| `BatchReader`, `RecordOptions` | [io](../io.md), [ipc](../ipc.md), [parquet](../parquet.md) |
| `schemaFromPattern` | [io](../io.md) |
| `iceberg` | [iceberg](../iceberg.md) |
| `MimeType`, `MediaType`, `Timezone` | [enums](../enums.md) |
| `codec`, `json`, `toml`, `yaml`, `Value` | [text](../text.md) and the format pages |

The compression codings are Rust-only today; a handle applies the one its name declares without
being told, so [gzip](../gzip.md), [zlib](../zlib.md), and [zstd](../zstd.md) are
reachable through `IOBase` even though their modules are not.

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
const { DataType, Field, MediaType, MimeType, Url } = require('yggdryl')
const assert = require('node:assert/strict')

// A datatype expression is a datatype.
assert.equal(String(new Field('id', 'int64', false).dataType), 'int64')
assert.equal(DataType.from('list<int32>').kind, 'list')

// A media type is its canonical name.
assert.equal(String(MimeType.from('application/json')), 'application/json')
assert.equal(String(MediaType.from('application/json')), 'application/json')

// A path is a location.
assert.equal(String(Url.fromPath('C:/tmp/a.json')), 'file:///C:/tmp/a.json')
```

There is no JavaScript-side parser: `DataType.from` and its siblings call the matching native
constructor.

## Values cross as their natural shape

A JavaScript value becomes the nearest native value, and comes back as the nearest JavaScript value
to that. No class name travels beside the data, so a shape the core does not have arrives as the
shape it was lowered to.

```javascript
const { json } = require('yggdryl')
const assert = require('node:assert/strict')

const decoded = json.loads(json.dumps({
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
| plain object, class instance | mapping | plain object |
| `Date` | timestamp, milliseconds, no zone | `Date` |
| `URL` | its `href` string | `string` |
| `RegExp` | its literal string, flags included | `string` |
| `DataType`, `Field`, `Uri`, `Url`, `Urn` | its canonical string | `string` |
| `Value` | itself | `Date` when one holds it exactly, otherwise `Value` |

These are the losses, and they are deliberate: a `Set` comes back a list, a `URL` and a `RegExp`
come back strings, a class instance comes back a plain object, a `Map` of text keys comes back a
plain object, and `undefined` comes back `null`. Reconstructing any of them takes one line of your
own code and needs no cooperation from the codec:

```javascript
const { json } = require('yggdryl')
const assert = require('node:assert/strict')

class Order {
  constructor(id) {
    this.id = id
  }
}

const decoded = json.loads(json.dumps({ order: new Order(7), venues: new Set(['XPAR']) }))
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

## Temporal and exact-decimal values

`Value` is the JavaScript spelling of what JavaScript has no type for: an exact decimal, a date, a
time of day, a duration, and a timestamp at a resolution or in a zone a `Date` cannot hold. A
`Date` is exactly a naive count of whole milliseconds, so `fromJs` reads one as that reading and
`asJs` hands one back. On the schemaless wires a temporal travels as its classic ISO string - the
fraction printed at the unit's full width, so the digits *are* the unit - and `loads` hands back
that string; a record class or a schema is what recovers the typed reading.

```javascript
const { Value, json } = require('yggdryl')
const assert = require('node:assert/strict')

const at = new Date('2026-08-15T12:30:00.000Z')
assert.ok(Value.fromJs(at).equals(Value.timestamp(1786797000000n, 'ms')))
assert.ok(Value.fromJs(at).asJs() instanceof Date)

// The wire spells the instant classically; the zone name rides in brackets.
const micros = Value.timestamp(1700000000123456n, 'us', 'UTC')
assert.equal(
  json.loads(json.dumps({ at: micros })).at,
  '2023-11-14T22:13:20.123456Z',
)
```

`Value.timestamp(count, unit, zone?)`, `Value.date(days)`, `Value.time(count, unit)`,
`Value.duration(count, unit)`, and `Value.decimal(unscaled, scale)` build one; `kind`, `count`,
`unit`, `zone`, `unscaled`, and `scale` read one back. A date counts days since the epoch and has
no unit; a decimal is `unscaled` times ten to the minus `scale`, which is the only representation
that round-trips, because `0.1` has no finite binary expansion.

```javascript
const { Value } = require('yggdryl')
const assert = require('node:assert/strict')

const price = Value.decimal(-1050n, 2) // -10.50
assert.equal(price.unscaled, -1050n)
assert.equal(price.scale, 2)

// Equality is what a value names, not how it was written.
assert.ok(price.equals(Value.decimal(-105n, 1)))
assert.ok(Value.duration(1n, 's').equals(Value.duration(1000n, 'ms')))
assert.equal(Value.date(19723).unit, null)
```

## fromJs and asJs

`Value.fromJs` and `Value.prototype.asJs` are the conversion pair. Every `load` and `dump` crosses
them - `dumps` is `fromJs` with bytes on the far side, `loads` is `asJs` - so calling them directly
is how you see what a value becomes before any format is involved.

```javascript
const { Value, json } = require('yggdryl')
const assert = require('node:assert/strict')

assert.equal(Value.fromJs(new Set([1, 2])).kind, 'sequence')
assert.deepEqual(Value.fromJs(new Set([1, 2])).asJs(), [1, 2])
assert.equal(Value.fromJs(new Map([['id', 1]])).kind, 'mapping')

const value = { id: 1, tags: new Set(['a']) }
assert.deepEqual(json.loads(json.dumps(value)), Value.fromJs(value).asJs())
```

Both accept the same `{ maxDepth }` the codec functions do, in the inclusive range 1 to 48.

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

## Records cross as one batch per stream

`BatchReader` is the one record shape: a read returns one and a write consumes one, exactly as
[`IOBase`](../io.md) does in Rust. `BatchReader.from` accepts whatever a caller already holds -
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
handle.writeArrowBatchReader(BatchReader.from(table))

const reader = handle.readArrowBatchReader()
assert.equal(reader.field.name, 'row')
assert.equal([...reader].reduce((rows, batch) => rows + batch.numRows, 0), 2)

// A stream is read once, and says so rather than reading as empty.
assert.ok(reader.consumed)
```

Each batch crosses as its own self-contained Arrow IPC stream, so its schema travels with it and
Arrow JS needs no separate handshake. That per-batch header is what a copied boundary costs, and it
is stated here rather than hidden. `toIpc` drains the reader into one stream and `toTable` into one
Arrow JS table, for the cases where a caller does want everything at once.

The encoding is never named by a call: `recordOptions()` derives it from the handle's media type, and
`RecordOptions` carries the shared settings plus the Parquet-only ones, which read as `null` on an
encoding that has none.

```javascript
const assert = require('node:assert/strict')
const { RecordOptions } = require('yggdryl')

const parquet = RecordOptions.from('trades.parquet')
assert.equal(String(parquet.mimeType), 'application/vnd.apache.parquet')
assert.equal(parquet.compression, 'zstd(1)')
assert.equal(parquet.withCompression('snappy').compression, 'snappy')

// A setting one encoding has is absent on the others rather than invented.
const stream = RecordOptions.from('trades.arrows')
assert.equal(stream.compression, null)
assert.equal(stream.maxRowGroupSize, null)
```

## Anything in, a reader out

`readArrow`, `writeArrow`, and `appendArrow` are the same three calls with the argument widened to
whatever your last library handed you. Each one becomes the single native reader and is passed to the
same native method, so widening the argument never adds a second way to write.

```javascript
const assert = require('node:assert/strict')
const arrow = require('apache-arrow')
const { IOBase, MimeType } = require('yggdryl')

const table = new arrow.Table({
  id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
})

function handle() {
  const stream = IOBase.fromBytes()
  stream.mediaType = MimeType.ARROW_STREAM
  return stream
}

// A table, a reader, named columns, and plain records all write.
for (const rows of [
  table,
  arrow.RecordBatchReader.from(arrow.tableToIPC(table)),
  { id: [1n, 2n] },
  [{ id: 1n }, { id: 2n }],
]) {
  const target = handle()
  target.writeArrow(rows)
  assert.equal(target.readArrow().toTable().numRows, 2)
}
```

| You are holding | What happens |
| --- | --- |
| a native `BatchReader` or Arrow IPC bytes | used as it stands |
| an Arrow JS `Table`, `RecordBatch`, or `RecordBatchReader` | its batches, encoded as one stream |
| an Arrow JS `Vector` | the one column a declared schema names, and refused when none does |
| an object of named columns | `tableFromArrays` |
| an object of scalar values | one row |
| an array or iterable of any of those | concatenated into one stream |
| an array of plain records | `tableFromJSON`, inferred from all of them at once |

Arrow JS has no C Data consumer, so this boundary encodes one Arrow IPC stream in both directions -
the batches were going to be materialized either way, which is why an array and a generator cost the
same here and why the Python side can stream where this one cannot.

An **async** source is the one shape that changes the call's shape: its rows do not exist until they
are awaited, so `writeArrow` returns a promise for it and nothing for every synchronous source. An
Arrow JS reader implements both iteration protocols and is treated as the synchronous one.

```javascript
const assert = require('node:assert/strict')
const arrow = require('apache-arrow')
const { IOBase, MimeType } = require('yggdryl')

async function main() {
  const table = new arrow.Table({ id: arrow.vectorFromArray([1n], new arrow.Int64()) })
  async function* pages() {
    yield table
    yield table
  }

  const handle = IOBase.fromBytes()
  handle.mediaType = MimeType.ARROW_STREAM
  await handle.writeArrow(pages())

  assert.equal(handle.readArrow().toTable().numRows, 2)
}

main()
```

`apache-arrow` is loaded only when a value actually has to be materialized, and a build without it
reports that package by name rather than failing somewhere inside a conversion.

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
assert.equal(table.scan().toTable().numRows, 2)

fs.rmSync(path.dirname(root), { recursive: true, force: true })
```

`iceberg.Table`, `iceberg.Catalog`, `iceberg.PartitionSpec`, and `iceberg.DataFile` are the
classes; `iceberg.assignFieldIds`, `iceberg.canPromote`, `iceberg.schemaFromJson`, and
`iceberg.schemaToJson` are the functions. A snapshot and a manifest arrive as plain objects,
because they are records of what happened rather than values with behaviour, and a 64-bit
identifier crosses as a `bigint` so a snapshot id past 2^53 is exact.

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
assert.deepEqual(catalog.listTables('nyc'), ['nyc.trades'])
assert.equal(table.scan().toTable().numRows, 3)

// A column change is a chain recorded on the update, committed once.
table.updateSchema().addColumn('', 'price: float64').commit()
assert.equal(table.scan().toTable().getChild('price').get(0), null)

// Undersized files rewrite as one replace commit that reports itself.
const compaction = table.compact()
assert.equal(compaction.filesBefore, 2)
assert.equal(compaction.filesAfter, 1)
assert.equal(table.scan().toTable().numRows, 3)

// And nothing rewrote history: the first snapshot reads as it was written.
assert.deepEqual(
  table.scanAt(past).toTable().getChild('id').toArray(),
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
[JavaScript](../notebooks/extensions_javascript-javascript.ipynb){ download }.

<!-- /notebooks -->
