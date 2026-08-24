# Yggdryl for JavaScript

```javascript
const { json, toml, yaml } = require('yggdryl')

const bytes = json.dumps({ symbol: 'AAPL', price: 224.62 })
const trade = json.loads(bytes)
const config = toml.loads('symbol = "AAPL"\nprice = 224.62\n')
const documents = yaml.loadsAll(Buffer.from('---\nsymbol: AAPL\n---\nsymbol: MSFT\n'))

console.assert(Buffer.isBuffer(bytes))
console.assert(trade.symbol === 'AAPL')
console.assert(config.symbol === 'AAPL')
console.assert(documents[1].symbol === 'MSFT')
```

JSON, YAML, and TOML emit only each format's natural shapes, with no private
tags. Exact decimals and binary values use ordinary strings where needed;
values with no natural spelling are refused. Pass `{ field }` while decoding
to restore exact widths and types in Rust. A source string is document content;
use a `file:` URL or descriptor for a location. Native path I/O avoids an
existence probe and JavaScript whole-file staging. Streams preserve
backpressure and remain caller-owned.

```javascript
const { Scalar, fields, json } = require('yggdryl')

const price = Scalar.d256(123456789012345678901234n, 4)
const field = fields.decimal256('price', 40, 4, { nullable: false })
const restored = json.loads(json.dumps(price), { field })

console.assert(restored.equals(price))
console.assert(price.kind === 'd256')
console.assert(typeof price.stableHash() === 'bigint')
console.assert(price.clone().compare(price) === 0)
console.assert(price.asJsonUtf8() === '"12345678901234567890.1234"')
```

`Scalar` factories retain `f16`/`f32`/`f64`, `d128`/`d256`, `date32`/`date64`,
`time32`/`time64`, `datetime64`, and `duration32`/`duration64`. Plain objects
become sorted named Records; JavaScript `Map` remains a Mapping.
Temporal factories take `(count, unit, timezone)`, accept a timezone name or
native `Timezone`, and use the explicit non-null `NAIVE` marker when omitted.
Immutable native values expose `equals`, `compare`, `stableHash`, and `clone`
whenever the Rust value has those semantics. JavaScript has no object hash
protocol, so `stableHash()` is the explicit deterministic `bigint`; JavaScript
`===` remains reference identity. `Expression` and `Statement` expose the
same native total structural order rather than a binding-side approximation.
`RecordOptions` exposes the same four methods over its encoding variant and
every current core setting; a clone is detached, so later mutation changes
only that copy's equality, order, and hash.
`Scalar.add`, `subtract`, `multiply`, `divide`, `remainder`, `negate`, and
`absolute` are checked native numeric operations. A plain JavaScript operand crosses
`Scalar.fromJs` exactly once before the operation; containers and text are not
silently concatenated or coerced. Invalid operands throw `TypeError` with
`ERR_YGGDRYL_INVALID_ARITHMETIC`; overflow, zero division, and inexact decimal
results throw `RangeError` with distinct `ERR_YGGDRYL_ARITHMETIC_OVERFLOW`,
`ERR_YGGDRYL_DIVISION_BY_ZERO`, and `ERR_YGGDRYL_INEXACT_ARITHMETIC` codes.
Expression methods with the same names build
lazy native expression nodes instead of evaluating them; an `Expression` stays
native, a string parses as expression text, and another JavaScript value is
inferred once as a literal. Avro `fingerprint` follows Parsing Canonical Form,
while `equals`, `compare`, and `stableHash` retain logical annotations,
defaults, aliases, and extension attributes.

```javascript
const { avro } = require('yggdryl')

const schema = { type: 'array', items: 'long' }
const bytes = avro.dumps([[1, 2], [3]], schema, { source: 'readme' })
const blocks = avro.blocks(bytes, {
  maxDepth: 8,
  maxInputBytes: 1_048_576,
  maxNodes: 10_000,
})

console.assert(blocks.metadata.source === 'readme')
console.assert(blocks.next().value.rows().length === 2)
```

The frozen `avro` namespace exposes native schemas, whole containers,
single-object framing, and a fused lazy compressed-block iterator. Optional
reader resolution and all decode limits cross in the same options object;
natural values still pass through the shared native `Scalar` conversion. A
whole decoded container is a detached `{schema, metadata, rows}` JavaScript
projection for natural destructuring and JSON use; only its native `schema`
member carries value protocols. Compressed blocks and their iterator are
one-shot operational objects.

```javascript
const { Readable, Writable } = require('node:stream')
const { json } = require('yggdryl')

async function main() {
  const value = await json.load(Readable.from(['{"id":', '42}']))
  await json.dump(value, new Writable({
    write(chunk, _encoding, done) {
      process.stdout.write(chunk, done)
    },
  }))
}

main()
```

`loadAll` redirects an async byte source to the lazy `loadAllStream` iterator;
`dumpAll` redirects a writable destination to `dumpAllStream`. Single-document
stream reads use one bounded buffer because Node's async reader cannot be
passed to synchronous Rust `Read`; multi-document reads and writes stay
incremental. That async boundary retains only document framing and
Node/WHATWG backpressure: each complete JSON Lines row or YAML document still
crosses the native codec for syntax, values, and local byte positions, with
parity tests against buffered core decoding. Iterators stop after their first
read or parse failure. Buffer, ArrayBuffer, SharedArrayBuffer, DataView, and
typed-array offsets are all preserved exactly.

TOML is deliberately single-document and exposes only `loads`, `load`,
`dumps`, `dump`, `loadStream`, and `dumpStream`. Generic `codec` operations
infer `.toml` paths in Rust or accept explicit `format: 'toml'` for content.

```javascript
const { DataType, Field, MediaType, MimeType, Uri } = require('yggdryl')

const type = DataType.fromString('struct<symbol:string,price:decimal(18,4)>')
const coarseClock = DataType.time('milliseconds')
const preciseClock = DataType.time('nanoseconds')
const field = new Field('trade', type, false, { source: 'book' })
field.setTableName('trades')
field.setParquetFieldId(17)
field.setLocation('s3://warehouse/trades/data.arrow')
field.setProperty('postgres', 'type', 'jsonb')
const file = Uri.fromPath('C:\\data\\trades.arrow')
const encoded = Uri.fromString('https://example.test/trades.csv.gz')
const partition = Uri.fromString('s3://warehouse/trades').joinPath('day=2026-08-23')

console.assert(Field.fromString(field.toString()).equals(field))
console.assert(coarseClock.kind === 'time32')
console.assert(preciseClock.kind === 'time64')
console.assert(field.parquetFieldId === 17)
console.assert(field.get('PARQUET:field_id') === '17')
console.assert(field.location.scheme === 's3')
console.assert(field.getProperty('postgres', 'type') === 'jsonb')
console.assert(file.toString() === 'file:///C:/data/trades.arrow')
console.assert(file.intoPath() === 'C:/data/trades.arrow')
console.assert(partition.toString() === 's3://warehouse/trades/day=2026-08-23')
console.assert(encoded.mediaType.base.equals(MimeType.fromString('text/csv')))
encoded.setMediaType(MediaType.fromParts('application/json', ['application/gzip']))
console.assert(encoded.fileName === 'trades.json.gz')
```

The Node-API package exposes native `DataType`, `Field`, `IOBase`, `MimeType`,
`MediaType`, `Timezone`, `Uri`, `Url`, `Urn`, and `Scalar` values, the record pair
`BatchReader` and `RecordOptions`, and the frozen `avro` and `iceberg`
namespaces over all of them.
Recursive parsing, validation, Arrow casting, structured scalar conversion, codec
conversion, ordering, stable hashing, metadata, and path normalization stay in
Rust.
`DataType.time(unit)` and `fields.time(name, unit)` require an explicit unit;
seconds/milliseconds select Time32 and microseconds/nanoseconds select Time64.
They accept the same native unit aliases as datatype parsing and reject
interval layouts.

```javascript
const { fields, intoField } = require('yggdryl')

let builds = 0
class Trade {
  static get intoStructField() {
    builds += 1
    return fields.struct('Trade', [
      fields.int64('id'),
      fields.utf8('symbol'),
      fields.decimal128('price', 18, 4),
    ], { nullable: false })
  }
}

const trade = intoField(Trade)
console.assert(intoField(new Trade()) === trade)
console.assert(builds === 1)
console.assert(trade.dataType.kind === 'struct')
```

`intoField(value, name?)` is the one dynamic field converter. A class exposes
its root through the actual static getter `intoStructField`; the loader validates
both its property descriptor and its non-null native Struct `Field` result, then
memoizes that result per class. Passing a different `name` returns a renamed
clone and leaves the cached root untouched.

Apache Arrow JS conversion is an explicit copied IPC boundary. `Scalar` exposes
`fromArrowScalar`, `fromArrowArray`, `fromArrowRecordBatch`, and
`fromArrowTable`, with matching `intoArrow*` methods; an optional native
`Field` selects and casts through the Rust Arrow engine. Empty or positional
values need a Field when their schema cannot be inferred. A `BatchReader`
read yields Arrow JS batches, and a write consumes a native reader, Arrow JS
table/batch/reader, IPC bytes, named columns, or plain rows through that same
reader path. `Field.castArrow` and `Field.cast` route holders through the native
cast engine with `{ safe: true }` by default. Run `npm run bench:records` for
the copied-IPC read, projection, cast, and write paths.

```javascript
const { fields } = require('yggdryl')

const payload = fields.struct('payload', [
  fields.int32('count', { nullable: false }),
  fields.utf8('note', { nullable: true }),
], { nullable: false })
const value = payload.defaultJSValue()

console.assert(value[0] === 0 && value[1] === null)
console.assert(payload.defaultJSHint().constructor === Array)
console.assert(fields.int32('id', { nullable: false }).defaultArrowScalar() === 0)
console.assert(
  fields.uint8('small').intoSchemeCompat('spark').dataType.kind === 'int16',
)
```

Defaults are computed by the Rust core and projected into natural JavaScript
values. A Struct is one ordered array with one value per child; its cached hint
names `Array` without materializing a default value or a second Field model.
Hint identity is wrapper-local, not a global interning promise for equivalent
fields.
Arrow scalars use one copied one-row IPC projection into Apache Arrow JS, and
unsupported Arrow JS layouts fail without introducing a second scalar model.
Run `npm run bench:defaults` for JavaScript defaults, cached hints,
compatibility normalization, and Arrow-scalar IPC materialization.

```javascript
const { Field, MediaType, MimeType } = require('yggdryl')

const media = MediaType.fromParts(
  MimeType.CSV,
  new Set([MimeType.GZIP, MimeType.ZSTD]),
)
const field = new Field('payload', 'binary', false, {
  'HTTPS:Content-Type': 'text/csv; charset=utf-8',
})
field.setMediaType(media)

console.assert(field.mimeType.equals(MimeType.CSV))
console.assert(field.contentEncoding === 'gzip, zstd')
console.assert(field.get('HTTPS:CONTENT-TYPE') === 'text/csv')
```

`MimeType` exposes every native known value as a frozen class constant and
accepts custom validated MIME names. `MediaType()` defaults to octet-stream;
its iterable adapters consume once and `encodings` returns a detached Array.
HTTP and HTTPS Field input shares canonical lowercase `http:*` Arrow metadata.
Raw headers preserve parameters, while typed MIME/media, exact unsigned
content length (`bigint`), and absolute HTTP Location accessors remain native.
Media pair updates are atomic and cache-aware.

```javascript
const { DataType, Field, fields } = require('yggdryl')

const id = fields.int32('id', { nullable: false })
const tags = fields.list('tags', fields.utf8('item'))
const row = DataType.fromFields([id, tags])

console.assert(id instanceof Field)
console.assert(tags.dataType.kind === 'list')
console.assert(row.kind === 'struct')
console.assert(id.showDiff(id) === '✓ equal')
console.assert([...id.showDiffs(fields.int64('id'))].length > 0)
```

The `fields` namespace covers every datatype variant and returns the same
generic native `Field`; TypeScript retains the exact kind/value view. Equality
can ignore recursive metadata with `equals(other, false)`, while `showDiffs`
returns a UTF-8 iterator and `showDiff` joins its terminal-readable lines.

Finite alternatives can use `DataType.variant(fields)` or
`fields.denseUnion(name, fields, options)`. Both assign dense Arrow Union type
IDs in declaration order and retain the ordinary `union` kind; union values
use the explicit `{ typeId, value }` shape. Bare `DataType.variant()` and
`fields.variant(name, options)` are different - the parenthesis disambiguates:
they build the first-class self-describing Variant datatype, which the Parquet
writer stores as its `VARIANT` logical type. PostgreSQL `json/jsonb` remains an
explicit adapter format, not an alias for either.

```javascript
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const arrow = require('apache-arrow')
const { Field, IOBase, fields, iceberg } = require('yggdryl')

// A location is absolute: a handle is a URL, and a relative name names no root.
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-'))

const schema = iceberg.assignFieldIds(
  fields.struct('row', [Field.from('id: int64'), Field.from('venue: utf8')], {
    nullable: false,
  }),
)
const rows = new arrow.Table({
  id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
  venue: arrow.vectorFromArray(['XNAS', 'XNYS'], new arrow.Utf8()),
})

// A handle reads and writes records through the encoding its name declares.
const file = new IOBase(path.join(root, 'trades.parquet'))
file.overwriteArrowTable(rows)
console.assert(file.readArrowReader().intoTable().numRows === 2)
console.assert(file.isIo())
console.assert(file.rowSize === 2)
console.assert(file.columnSize === 2)

// An Iceberg table is a folder, and a folder is all it ever touches.
const table = iceberg.Table.create(path.join(root, 'trades'), schema, ['venue'])
table.append(rows)
console.assert(table.currentSnapshot.operation === 'append')
console.assert(table.dataFiles().length === 2)
```

`PartitionSpec`, `PartitionField`, `Snapshot`, `SnapshotRef`, `ManifestFile`,
`DataFile`, and `Compaction` are immutable native values with `equals`,
`compare`, `stableHash`, and `clone`. A `DataFile`'s partition spec
only names its tuple positions and is not part of the core file identity;
cloning retains that projection context. `PartitionSpec.fromJSON` accepts the
v1 field array or v2 object, while `intoJSON` and the standard `toJSON` hook
emit the v2 object through the shared native `Scalar`. `IcebergOptions` exposes
the same four value methods over its explicitly configured settings; a clone
is detached, and mutation naturally changes equality, order, and hash.
`ScanPlan` is an immutable bounded report with the same four methods. Its
complete identity, in comparison order, is `(recordCount, filesPlanned,
filesSkipped, manifestsRead, manifestsSkipped)`; physical task paths are not
part of the public report.
The other classes retain their complete Rust-core values, including snapshot
v3 lineage and manifest partition summaries, so their protocols never compare
a lossy JavaScript projection. Exact 64-bit identifiers cross as `bigint`.

`BatchReader` is the primitive record shape: `readArrowReader` returns one and
the `overwriteArrowReader`/`appendArrowReader`/`mergeArrowReader` methods consume
one. Table, record-batch, and record methods infer their input then redirect to
those primitives. Each Arrow JS crossing is copied IPC because Arrow JS has no
C Data consumer. The encoding is never an argument: `recordOptions()` derives
it from the handle's media type, covering Arrow IPC and Apache Parquet.

Configured intent uses `writeArrowReader`, `writeArrowTable`,
`writeArrowRecordBatch`, or `writeRecords` with `(input, mode, options?)`.
The required mode is `'overwrite'`, `'append'`, or `'merge'` and is checked
before a one-shot reader, exporter, or iterable is inspected.

`isIo()` is the general capability check: byte values and tabular media return
true, while a container holding neither returns false. `rowSize` and
`columnSize` are lazy metadata getters for the whole logical media, independent
of projections and read limits. Successful answers are cached by the Rust core
only while a handle is open, invalidated by writes through that handle, and
computed fresh after `close()`; JavaScript keeps no parallel count or schema.
Counts saturate at `Number.MAX_SAFE_INTEGER` rather than becoming imprecise.

Record iterables cross in bounded IPC chunks (`options.batchSize`, or 1,024
rows). Synchronous iterables are pulled lazily through one native reader. With
no `options.commitRowSize`, async iterables spool those bounded chunks to a
private temporary file before the one native write, preserving one publication
while bounding memory; the spool is removed on success or failure.

With a positive `options.commitRowSize`, synchronous and asynchronous chunks
end at the exact cadence boundary even when it does not divide `batchSize`.
The async path alternates one awaited chunk with one opaque Rust-session push,
so every complete prefix is visible before the next source pull and a later
failure drops only the incomplete cadence. Global row and byte limits remain
one operation-wide budget. A zero limit does not inspect the source: append is
a synchronous no-op, overwrite publishes an explicitly typed empty value, and
a limited merge is rejected.

Use the complete [JavaScript guide](https://platob.github.io/yggdryl/extensions/javascript/)
for field conversion, typed rows, copied IPC interoperability, and codec
format inference.
