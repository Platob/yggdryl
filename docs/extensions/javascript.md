# JavaScript

What the Node-API package adds on top of the [core](../index.md), and how
values cross the JavaScript boundary.

## Contract

| Name | Documented in |
| --- | --- |
| `DataType` | [datatype](../types/datatype.md) |
| `Field`, `fields` | [field](../types/field.md) |
| `StringEnum`, `StringParameters`, `BytesParameters` | [strings & bytes](../types/text.md), [codes](../types/codes.md), and this page |
| `Version` | [numeric versions](../types/text.md#versions) and this page |
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
| `txhash`, `TxHash`, `TxHasher` | [txhash](../txhash/index.md) |

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

## Numeric versions

`new Version(major, minor = 0, patch = 0)` constructs the native four-byte value.
Major and minor accept exact integers in `0..255`, patch in `0..65535`;
fractions, non-finite numbers and overflow are refused at the native boundary.
`fromStr` reads one to three decimal components, and a compact FIX service
pack - `5.0SP2` is `5.0.2` - case-insensitively. The major and minor are
strict; a patch tail stating no number folds into the patch rather than
throwing, so only empty text and a bad major or minor throw. All three
properties are read-only, and the value stores no tag or qualifier.

```javascript
const assert = require('node:assert/strict')
const { Scalar, Version, fields, json } = require('yggdryl')

const version = Version.fromStr('005.0.00300')
assert.ok(version.equals(new Version(5, 0, 300)))
assert.equal(version.toString(), '5.0.300')
assert.deepEqual([version.major, version.minor, version.patch], [5, 0, 300])
assert.equal(new Version(5, 0, 2).compare(new Version(5, 0, 10)), -1)
assert.equal(new Version(5).stableHash(), Version.fromStr('5.0.0').stableHash())
assert.ok(version.clone().equals(version))
assert.ok(Scalar.fromJs(version).asJs().equals(version))

const field = fields.version('release', { nullable: false })
const scalar = Scalar.fromJs('5.0.300', { field })
assert.ok(scalar.asJs() instanceof Version)
assert.equal(scalar.intoArrowScalar(field), '5.0.300')
assert.ok(json.loads('"5.0.300"', { field }).equals(version))
assert.equal(JSON.stringify(version), '"5.0.300"')
```

`VersionField` declares this native value. Arrow retains canonical `Utf8`
storage and the `yggdryl.version` extension; its string sorting stays
lexicographic. Use `compare` for numeric ordering and `stableHash()` for the
native deterministic `bigint` hash. [Version measurements](../types/text.md#performance)
include the parser, comparison and Scalar boundary.

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
| `Version` | native four-byte numeric version | `Version` through `Scalar.asJs()` or a declared Version field; canonical text in schemaless JSON |
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
`asBytes`/`asStr` borrow content - `asStr` answers a string, a code, or an
enum member - and `asJsonBytes`/`asJsonUtf8` use the core natural JSON writer.

## Strings and bytes at the boundary

The one string family and the one byte family of [strings & bytes](../types/text.md) cross as plain objects: `DataType.string({ layout, charset, bound | fixed | max })` and `DataType.bytes({ layout, bound | fixed | max })` read a declaration, the layout statics (`utf8`, `largeUtf8`, `utf8View`, `ascii`, `fixedUtf8(width)`, `fixedAscii(width)`, `binary`, `largeBinary`, `binaryView`, `fixedSizeBinary(width)`) pick one, and `stringParameters` / `bytesParameters` answer it back with `bound` beside its reading - `fixed` on the fixed layout, `max` everywhere else - and no key at all when the layout gives no such reading. `fields.string(name, options)` and `fields.bytes(name, options)` split one options object: the parameter keys build the datatype, every other key is a field option. `Field.stringEnum` / `setStringEnum` carry a `StringEnum`, accepted on a fixed US-ASCII string of at most sixteen bytes or a code and refused by name elsewhere.

```javascript
const assert = require('node:assert/strict')
const { DataType, Field, Scalar, StringEnum, fields } = require('yggdryl')

// A declaration is one datatype; the statics are the declaration with the
// layout and charset picked once.
const declared = DataType.string({ layout: 'string', charset: 'windows-1252', max: 8 })
assert.equal(declared.toString(), 'string(windows-1252,8)')
assert.deepEqual(declared.stringParameters, {
  layout: 'string',
  charset: 'windows-1252',
  bound: 8,
  max: 8,
})
assert.ok(DataType.utf8().equals(DataType.string({})))
assert.deepEqual(DataType.utf8().stringParameters, { layout: 'string', charset: 'utf-8' })
assert.equal(DataType.fixedAscii(4).stringParameters.fixed, 4)
assert.equal(DataType.fixedAscii(4).fixedByteWidth, 4)
assert.equal(DataType.from('ascii(4)').stringParameters.max, 4)
assert.equal(DataType.bytes({ max: 16 }).bytesParameters.max, 16)
assert.equal(DataType.fixedSizeBinary(16).bytesParameters.fixed, 16)

// The field factories take the bound by its reading beside the field options.
const name = fields.string('name', { charset: 'us-ascii', max: 8, nullable: false })
assert.equal(name.dtype.toString(), 'ascii(8)')
assert.equal(name.nullable, false)
assert.equal(fields.fixedUtf8('code', 4).dtype.toString(), 'fixed_utf8(4)')
assert.equal(fields.bytes('blob', { max: 16 }).dtype.toString(), 'binary(16)')
assert.throws(() => fields.string('both', { fixed: 4, max: 8 }), /one of bound, fixed and max/)

// A value never carries a maximum; a code is its own kind.
assert.equal(Scalar.fromJs('AAPL').kind, 'string')
assert.equal(Scalar.fromJs('AAPL').asStr(), 'AAPL')
assert.equal(new DataType('currency').kind, 'code')

// A vocabulary is metadata on a fixed US-ASCII string or a code.
const side = new Field('side', DataType.fixedAscii(4), false)
side.setStringEnum(new StringEnum('Side', { BUY: 'B', SELL: 'S' }))
assert.equal(side.stringEnum.get('BUY'), 'B')
assert.equal(side.stringEnum.intoMembers(side.dtype).BUY, 0x42000000n)
assert.throws(
  () => new Field('side', DataType.fixedUtf8(4)).setStringEnum(new StringEnum('Side', { BUY: 'B' })),
  /fixed US-ASCII string of at most 16 bytes/,
)
```

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

`Expression` exposes the same names except `absolute`, as lazy tree builders. A
string operand is parsed as expression text, any other value as a literal.

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
assert.equal(tree.set('venue', 'XNAS').get('venue').asStr(), 'XNAS')
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
field.digest.set('role', 'holder')
field.identity.update({ role: 'primary', nulls: 'distinct' })
field.partition.update({ transform: 'year', sources: '["event"]' })

assert.equal(field.iceberg.get('doc'), 'closing price')
assert.deepEqual([...field.postgres], [['type', 'numeric']])
assert.equal(field.iceberg.size, 1)
assert.equal(field.postgres.has('doc'), false)
assert.equal(field.digest.get('role'), 'holder')
assert.equal(field.identity.get('role'), 'primary')
assert.equal(field.partition.get('transform'), 'year')

// The bare name is all the view needs; the full key is what the field stores.
assert.equal(field.iceberg.key('doc'), 'iceberg:doc')
assert.equal(field.get('iceberg:doc'), 'closing price')
assert.equal(field.size, 7)

assert.equal(field.iceberg.delete('doc'), true)
assert.equal(field.iceberg.size, 0)
```

`digest`, `identity`, `partition`, and `python` join the well-known protocol
getters, and `field.protocol(name)` takes a runtime scheme. There is no `https`
getter, because HTTPS shares the canonical `http:` namespace.

`field.python` carries the class a [Python](python.md#the-declaring-class)
schema was built from. JavaScript reads and writes its three properties by
name; the typed vocabulary over them is Rust and Python only, so the eight
spellings `python:kind` accepts are `enums.pythonKinds`.

```javascript
const assert = require('node:assert/strict')
const { Field, enums } = require('yggdryl')

const quote = new Field('Quote', 'int64', false)
quote.python.update({
  kind: 'dataclass',
  module: 'trading.book',
  qualname: 'Book.Quote',
})

assert.equal(quote.get('python:qualname'), 'Book.Quote')
assert.deepEqual(quote.python.keys(), ['kind', 'module', 'qualname'])
assert.ok(enums.pythonKinds.includes('dataclass'))

// The core validates every one of them, and a refusal changes nothing.
assert.throws(() => quote.python.set('kind', 'record'), /python:kind/)
assert.equal(quote.python.get('kind'), 'dataclass')
```

A schema also says which columns a path spells out.

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

## Row digest fields

A Struct field's direct children define a row digest. Explicit `component`
roles select the exact set, otherwise every child except a `holder`
contributes, both in declaration order.

```javascript
const { DataType, Field } = require('yggdryl')
const assert = require('node:assert/strict')

const identifier = new Field('id', 'int64', false)
const price = new Field('price', 'int64', false)
const holder = new Field('row_digest', 'uint64', false)
holder.digest.set('role', 'holder')

const fallback = new Field(
  'row', DataType.fromFields([identifier, price, holder]), false,
)
assert.deepEqual(fallback.digestFieldNames(), ['id', 'price'])
assert.equal(fallback.digestFieldLen, 2)

// A holder narrows its own input; the fields it reads stay unmarked.
holder.digest.set('sources', '["id"]')
assert.deepEqual(identifier.digest.entries(), [])
assert.equal(holder.digest.get('sources'), '["id"]')
assert.equal(fallback.onlyDigestFields().dtype.length, 2)
```

Digest holders accept `int32`/`uint32` for XXH32 and `int64`/`uint64` for the
64-bit algorithms. `field.castArrowArray(values, { representation: 'bits' })` is
the same reversible [same-width reading](../types/cast.md#reading-the-bits)
outside holder filling.

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

Each resumable state also exposes `applyArrowBatch(root, batch, force = false)`.
The root Field's digest metadata selects the row values, and the state supplies
its algorithm, seed, and secret.

`txhash` couples an instant with that digest. Every `unix` argument is a
`bigint`, an integer `number`, a `Date`, timestamp text, or a `Scalar`; the
coupled columns stay Rust and Python only ([txhash](../txhash/index.md)).

```javascript
const assert = require('node:assert/strict')
const { TxHash, txhash, xxhash } = require('yggdryl')

const value = txhash.txh3('abc', new Date('2023-11-14T22:13:20Z'))
assert.equal(value.unix, 1_700_000_000_000_000n)
assert.equal(value.digest.value(), xxhash.xxh3('abc'))
assert.equal(Buffer.from(value.bytes()).readBigInt64BE(0), value.unix)
assert.ok(TxHash.from(value.toString()).equals(value))
```

## FIX is a namespace

`fix.FixRegistry`, `fix.FixMsg`, `fix.MsgType`, `fix.FixCodec`,
`fix.FixMessages`, `fix.FixLifecycle`, `fix.UlPlugin`, `fix.UlPlugins`,
`fix.schema()`, `fix.schemaCarrying()`, `fix.schemaTags()`, `fix.crateFields()`,
`fix.ulbridgeFields()`, `fix.globalRegistry()` and `fix.installGlobalRegistry()`
are the whole surface: the registry, message definitions, codec, messages and
lazy iterators. The namespace holds no constant: a dictionary is one namespace
of tags and names, an identity is the number `field.fix.id` derives from both,
and a dictionary's membership is `fix:branches` on the field it contributed
to. The `fix:` vocabulary is typed accessor pairs on the `field.fix` view,
including `id`, `tag`, `tags`, `aliases`, `branches`, `description`, `codes`,
`counter`, `component` and `msgtype`.

| Crossing | Rule |
| --- | --- |
| tag key | a `number`, coerced once and checked exactly |
| identifier | a `number`: the signed 32-bit digest of a tag and a name, what `field.fix.id` answers; `fieldById`, `getFieldById`, `removeById`, `message.byId` and `getById` take it exactly - no fold, no tiering - and a fractional or out-of-`i32` number is refused rather than narrowed. A bare number anywhere else is a tag, never an identifier |
| `FixMsg.arrivals()` | `[tag, key, value]` tuples, flattened pre-order, so a group's members follow the counter pair heading them |
| name or path key | a `string`, folded once - ASCII case, `_`, `-` and space dropped; a bare string is a name, never an identifier |
| membership | `field.fix.branches` is a `string[]`, sorted and lowercase, `[]` where `fix:branches` is absent; assigning an array replaces the list, folded and deduplicated, and `[]` removes the property; `addBranch(name)` is idempotent under the fold and `hasBranch(name)` folds the same way; a name that is empty or carries a comma is refused. `registry.dialects()` lists the distinct names any field or definition carries. Membership is provenance a caller filters on; no lookup consults it |
| `fieldByName`, `fieldByPath` | one namespace, no branch argument: the canonical fold answers first, then an alias fold; a path is decided by the one grammar |
| `fieldByTag` | the canonical holder of a tag answers first, then the field holding it as an alternate |
| `field.fix.id` | derived on every read from `fix:tag` and the field's name, never stored, `null` exactly when `fix:tag` is absent; the property has no setter |
| `fromCfbFile(location, dialect?)` | answers `[registry, roots]`; `dialect` stamps every field, group, component and message the file produces on its `fix:branches`, standard tags included, and with none named nothing is stamped; the root element's version is read past |
| `message.at`, `message.byId` | the failing halves; `value` holds the whole message value |
| `fromHandle`, `writeInto` | an `IOBase`, a `Url`, or the string naming one |
| `FixCodec.lifecycle`, `FixLifecycle.fill` | take and answer `FixMsg` - any iterable in and a lazy `FixMessages` out for the codec, one at a time for the lifecycle; `FixLifecycle.alive` is a read-only number |
| iteration | registry tag-major, the tag's holder first, then by identifier; message in the root's declared order |
| categories | `fields`, `messages`, `components`, `groups`; enums stay inline in a field's `fix:codes` metadata, and a named definition carries the `fix:tag` derived from its name, in `[100000, 1100000)`, which a reference occurrence inside it never restates |
| CRUD | `createDefinition`, `definition`, `updateDefinition`, `removeDefinition`; `definitions` iterates one category lazily; `addField` and `addDefinition` are the lenient twins, answering `true` when the field or definition arrived and `false` when it folded into a stored one |
| `MsgType` | immutable registry-owned message Struct, borrowed through `msgtype` / `getMsgtype` or lazy `msgtypes`; complete UTF-8 wire code |
| `FixCodec` | pins cross in the options object - `version`, `separator`, `payloadColumn`, `captureNames`, `nullValues`, `direction`, `batchByteSize`; `parseLine`, `parseTextLine`, `parseUlconfigLine` return lazy `FixMessages`, `parseLines`, `parseTextLines`, `enrichMessages` and `messages` lazy `FixMsg` iterators; `parseFixLine`, `parseUllinkLine`, `parseFixmlLine`, `parsePairs` and `enrichMessage` answer one `FixMsg`; no reader takes a flag |
| Arrow twins | `parseTextArrowReader`, `enrichMessagesArrowReader` and `arrowReader(schema, messages)` take and answer a native `BatchReader`, so `BatchReader.from` widens an Arrow JS table on the way in and `intoTable` drains the answer; `writeArrowReader(reader, sink)` writes lines into anything with `write(chunk: Uint8Array)` and answers their count |
| `FixMsg` writes | `set(key, value)` and `remove(key)` change the row in place and never the entries; `FixMsg.fromRow(schema, row, registry)` reads a fixed row back, entries included |
| output | `FixMsg.intoRow(field)` projects a table row; `intoBytes(separator = 1)` re-emits ordered arrival pairs, empty for a message built without arrivals |
| ULconfig | `UlPlugin.fromJsonBytes` / `fromJsonScalar` return lazy `UlPlugins`; each selection converts to one flat message with `intoFixmsg` |

```javascript
const assert = require('node:assert/strict')
const path = require('node:path')
const { Field, IOBase, Url, fields, fix } = require('yggdryl')

const seed = path.resolve('config/fix')

// One folder, named however JavaScript names one - the coercion `Catalog` uses.
const url = Url.fromPath(seed)
const registry = fix.FixRegistry.fromHandle(seed)
for (const location of [seed, url.toString(), url, new IOBase(seed)]) {
  assert.equal(fix.FixRegistry.fromHandle(location).size, registry.size)
}

// A key is a number tag or a string name, and a tag that would not fit i32 is
// refused rather than narrowed into a different one.
assert.ok(registry.get(55).equals(registry.get('symbol')))
assert.throws(() => registry.getFieldByTag(2 ** 31), /tag must be a signed 32-bit integer/)
assert.throws(() => registry.get(55n), {
  name: 'TypeError',
  message: 'key must be a number tag or a string name, got BigInt',
})

// Names are folded once, so a caller spells one however they have it.
assert.equal(registry.fieldByName('SYMBOL').name, 'symbol')
assert.equal(registry.fieldByPath('Parties.PartyID').fix.tag, 448)
assert.equal(registry.definition('fields', 'NoPartyIDs').dtype.id, 'int32')
assert.equal(registry.definition('groups', 'Parties').fix.counter, 453)
assert.equal(registry.definition('components', 'Party').dtype.kind, 'nested')

// An identifier is a number - the signed 32-bit digest of a tag and a name,
// what `field.fix.id` answers - and the `ById` doors take it exactly. A bare
// number anywhere else is a tag, a bare string a name, and a malformed
// identifier throws rather than missing.
const symbolId = registry.fieldByTag(55).fix.id
assert.ok(Number.isInteger(symbolId))
assert.equal(registry.fieldById(symbolId).name, 'symbol')
assert.equal(registry.getField(`${symbolId}`), null)
assert.throws(() => registry.fieldById(1.5), /id must be a signed 32-bit integer/)
assert.throws(() => registry.fieldById('55'), /into rust type `f64`/)

// Absence throws with the native message; the `get` half answers null.
assert.throws(
  () => registry.fieldByName('Nope'),
  /expected a fix field at "name/,
)
assert.equal(registry.getFieldByName('Nope'), null)
assert.throws(() => registry.insert(Field.from('Untagged: utf8')), /fix:tag/)

// The typed vocabulary is answered by the fix view alone. The identity is
// derived from the tag and the name on every read, never stored, and the name
// folds - ASCII case, `_`, `-` and space dropped - so two spellings of one
// field under one tag are one identity.
const symbol = registry.fieldByTag(55)
assert.equal(symbol.fix.tag, 55)
assert.equal(symbol.fix.id, symbolId)
assert.equal(symbol.has('fix:id'), false)
// The specification's own spelling stays on the generic display key.
assert.equal(symbol.display, 'Symbol')
assert.throws(() => symbol.iceberg.tag, { name: 'TypeError', message: /iceberg/ })
const vendor = Field.from('TradeID: utf8')
assert.equal(vendor.fix.id, null)
vendor.fix.tag = 5001
const vendorId = vendor.fix.id
const spelled = Field.from('trade_id: utf8')
spelled.fix.tag = 5001
assert.equal(spelled.fix.id, vendorId)
// The identity has no setter: an assignment does not take (strict code sees
// a `TypeError`), because the tag and the name are what it is made of.
vendor.fix.id = 42
assert.equal(vendor.fix.id, vendorId)

// Membership is provenance on the field, a sorted lowercase list under
// `fix:branches`; it changes no identity and no lookup consults it.
assert.deepEqual(vendor.fix.branches, [])
vendor.fix.branches = ['CME', 'cme']
assert.deepEqual(vendor.fix.branches, ['cme'])
assert.equal(vendor.get('fix:branches'), 'cme')
vendor.fix.addBranch('ICE')
assert.equal(vendor.fix.hasBranch('Ice'), true)
assert.equal(vendor.fix.id, vendorId)

// A message shares the dictionary it resolved against, so mutating it refuses.
const root = fields.struct('row', [symbol], { nullable: false })
const message = new fix.FixMsg(root, { symbol: 'AAPL' }, registry)
assert.throws(() => registry.remove(55), /shared with a message/)
const independent = fix.FixRegistry.fromFields([symbol])
assert.equal(independent.remove(55).name, 'symbol')

// One namespace: a venue reusing the tag 55 under another name stands beside
// its holder, which gains the name as an alias while the bare tag keeps
// answering it; the newcomer is reached by its name or its identifier, and
// `removeById` is how it leaves. `dialects()` lists what any field names.
const venue = fix.FixRegistry.fromFields([vendor])
const venueSymbol = Field.from('VenueSymbol: utf8')
venueSymbol.fix.tag = 55
venueSymbol.fix.branches = ['cme']
assert.equal(venue.insert(symbol), null)
assert.equal(venue.insert(venueSymbol), null)
assert.equal(venue.fieldByTag(55).name, 'symbol')
assert.deepEqual(venue.fieldByTag(55).fix.aliases, ['VenueSymbol'])
assert.equal(venue.fieldByName('venuesymbol').fix.id, venueSymbol.fix.id)
assert.deepEqual(venue.dialects(), ['cme', 'ice'])
assert.equal(venue.removeById(venueSymbol.fix.id).name, 'VenueSymbol')
assert.equal(venue.remove('TradeID').name, 'TradeID')
// What remains is the crate's own fields, which every registry holds.
assert.equal(venue.size, 1 + fix.crateFields().length)
assert.deepEqual(venue.dialects(), [])

// Both collections are lazy native iterators the loader gives the protocol.
assert.equal([...registry].length, registry.size)
assert.deepEqual([...message].map(([name]) => name), ['symbol'])
assert.equal(message.at('SYMBOL').asJs(), 'AAPL')
assert.equal(message.byId(symbolId).asJs(), 'AAPL')
assert.equal(message.getById(vendorId), null)

// Generic intake is lazy even when the source yields one message.
const wire = Buffer.from('8=FIX.4.4|35=D|55=AAPL|10=0|')
const messages = new fix.FixCodec(registry).parseLine(wire)
const parsed = messages.next().value
assert.equal(messages.next().done, true)
assert.deepEqual(parsed.intoBytes('|'.charCodeAt(0)), wire)
// A message root the codec builds is not a dictionary member.
assert.deepEqual(parsed.field.fix.branches, [])
const tableField = fix.schema(registry)
assert.equal(parsed.intoRow(tableField).asJs().length, tableField.fieldLen)
```

`FixMsg`'s constructor is the one widening gate: the core alone types, orders,
and validates a plain object. Resolution and merging are the core's, on the
[fix](../fix/index.md) pages.

Bulk configuration responses stream one flat message per selected plugin. Each
message retains its selected MBean and source envelope, with the plugin's
fields directly addressable on the message. The bridge's fields are members of
the `ulbridge` dictionary and resolve in the one namespace like any other; the
document's `State` and `Version` attributes are held under `PluginState` and
`PluginVersion`, because every registry already holds the crate's own `state`
and `version`, and the arrival record keeps the document's spelling.

```javascript
const assert = require('node:assert/strict')
const { fix } = require('yggdryl')

const registry = new fix.FixRegistry()
registry.withUlbridgeFields()
// The bridge's fields are members of the `ulbridge` dictionary; no codec pin
// names one, since the dictionary is one namespace.
assert.deepEqual(registry.dialects(), ['ulbridge'])
assert.deepEqual(registry.fieldByName('MBean').fix.branches, ['ulbridge'])
assert.equal(registry.fieldByName('PluginState').fix.tag, 20019)
assert.equal(registry.fieldByName('PluginVersion').fix.tag, 20021)
const codec = new fix.FixCodec(registry)
const document = [
  { request: { type: 'read', mbean: 'bridge:type=Plugin,name=Orders' },
    status: 200, value: { Name: 'Orders', State: 'Running', Version: '1.2' } },
  { request: { type: 'read', mbean: 'bridge:type=Plugin,name=Prices' },
    status: 200, value: { Name: 'Prices' } },
]
const messages = codec.parseUlconfigLine(Buffer.from(JSON.stringify(document)))
assert.ok(messages instanceof fix.FixMessages)
const [orders, prices] = [...messages]
assert.deepEqual([orders, prices].map(message => message.byName('Name').asJs()), ['Orders', 'Prices'])
// The document's `State` and `Version` attributes are held under the bridge's
// own names; the arrival entry keeps the document's spelling.
assert.equal(orders.byName('PluginState').asJs(), 'Running')
assert.equal(orders.byName('PluginVersion').asJs(), '1.2')
assert.deepEqual(
  orders.arrivals().filter(([tag]) => tag === 20019 || tag === 20021),
  [[20019, 'State', 'Running'], [20021, 'Version', '1.2']],
)
assert.equal(messages.next().done, true)
const selected = fix.UlPlugin.fromJsonScalar(document).next().value
assert.equal(selected.intoFixmsg(codec).byName('Name').asJs(), 'Orders')
```

## Edges

- Any native refusal -> `TypeError` or `RangeError` with the Rust message, path
  or byte offset included.
- Streaming `reader`/`writer`, the handle wrappers, `Hashed<H>`, and the
  per-protocol view types -> Rust-only.
- The typed `python:` vocabulary (`classMetadata` and its parts) -> Rust and
  Python only; `field.python` is the generic `Map` view here, and
  `enums.pythonKinds` is where the eight forms are listed.
- Rust spellings -> `as_iceberg()`/`as_iceberg_mut()`, `contentType` as
  `as_http().content_type()`, `arrow` as `as_arrow_properties`, and
  `fieldProperties` as `as_field_properties`.
- Rust field metadata -> `http:` headers on `as_http()`, while
  `parquet_field_id`, `alias`, `comment`, `display`, and `location` stay on
  `Field`.
- `applyArrowBatch` -> fills default holder cells; a populated holder is
  preserved unless `force` is true.
- `applyArrowBatch` -> ignores bytes already written to the state, leaves it
  unchanged, and copies Arrow batches as IPC in both directions.
- A signed digest holder -> high-bit results read as a negative `number` or
  `bigint`, with the complete digest bits retained.
- `field.digest` `role` -> only `'holder'` or `'component'`; `identity` and
  `partition` take arbitrary inert string metadata.
- `DataType.fromRegex` -> named-capture inference, decided by the core.
- `gzip`, `zlib`, `zstd` -> `loads`/`dumps` over `Buffer`, plus
  `loadsRaw`/`dumpsRaw` on `zlib`, reading and writing what `node:zlib` does.
- A handle -> applies the coding its name declares without being told;
  `IOBase.codec` asks which one that is.
- `TextOptions.startRownum` -> `bigint | null` over the whole signed 64-bit
  range; a `number` is rejected, never silently narrowed.
- `TextOptions` logical framing -> `framing`, `leadingFragment`, and
  `maxRecordByteSize`, contracted in [plain-text records](../media/text.md).
- A `bigint` wider than 128 bits -> refused, since no exact native integer
  holds it.
- A handler-backed handle in a `Worker` -> refused by name; handlers run
  synchronously on the thread that supplied them.
- `readRange` with an unknown option, a non-boolean `text`, or an undecodable
  range -> refused.
- `readRange` `length` -> checked as strictly as `offset`, never rounded.
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
- A synchronous record iterable -> one copied IPC chunk per native request, so
  the source is never held whole.
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
- The `fix:` vocabulary read or written on another protocol's view ->
  `TypeError` naming that view's scheme.
- `field.fix.id` -> a `number` in strict code and sloppy code alike; the
  property has no setter, so an assignment throws under `'use strict'` and
  does not take without it.
- `message.field.fix.branches` -> `[]` for a root the codec built: a message
  is not a dictionary member.
- `message.liftSource(facet)` -> the tag a lifted facet was read from, a
  `number`, or `null`; the field that tag names is the registry's to answer.
- Registry mutation while a `FixMsg`, `MsgType`, the process default, or a live
  native iterator holds it -> throws; `registry.clone()` is the mutable deep copy.
- A `keys()` walk -> stops sharing when drained, or when a `for...of` `break`
  returns it.
- `registry.remove(key)` -> a number is a tag and a string is a name;
  `removeById(id)` reaches one of two fields sharing a tag by its own
  identity, and answers `null` for one that is not there.
- FIX absence -> the native refusal, or `null` from the `get`-prefixed twins,
  for a key that parses.
- A missing FIX folder -> a registry holding only the crate's own fields.
- A registry write -> `fields/<shard>.json` with the shard a tag's hundred,
  and `messages/`, `components/` and `groups/` with every definition directly
  below its category; membership travels inside each field's metadata, and
  the crate's own fields are never written.
- `message.getById`/`byId` -> exact: no fold, no tiering; a field the
  dictionary does not hold under the identifier misses.

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
