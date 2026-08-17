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

JSON, YAML, and TOML APIs are byte-first and preserve binary
values, large integers, dates, maps, sets, and typed arrays. `load` and `dump`
also accept paths, descriptors, file URLs, and Node or WHATWG streams. Primitive
strings are content unless they name an existing regular file; `loads` always
treats them as content. Real paths redirect to Rust reader/writer methods
without a whole-file JavaScript buffer. Stream overloads are asynchronous and
apply writable backpressure without closing caller-owned I/O.

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
incremental. Buffer, ArrayBuffer, SharedArrayBuffer, DataView, and typed-array
offsets are all preserved exactly.

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

console.assert(Field.fromString(field.toString()).equals(field))
console.assert(coarseClock.kind === 'time32')
console.assert(preciseClock.kind === 'time64')
console.assert(field.parquetFieldId === 17)
console.assert(field.get('PARQUET:field_id') === '17')
console.assert(field.location.scheme === 's3')
console.assert(field.getProperty('postgres', 'type') === 'jsonb')
console.assert(file.toString() === 'file:///C:/data/trades.arrow')
console.assert(file.toPath() === 'C:/data/trades.arrow')
console.assert(encoded.mediaType.base.equals(MimeType.fromString('text/csv')))
encoded.setMediaType(MediaType.fromParts('application/json', ['application/gzip']))
console.assert(encoded.fileName === 'trades.json.gz')
```

The Node-API package exposes native `DataType`, `Field`, `IOBase`, `MimeType`,
`MediaType`, `Timezone`, `Uri`, `Url`, `Urn`, and `Value` values, the record pair
`BatchReader` and `RecordOptions`, and the `iceberg` namespace over all of them.
Recursive parsing, validation, Arrow casting, record scalar conversion, codec
conversion, ordering, stable hashing, metadata, and path normalization stay in
Rust.
`DataType.time(unit)` and `fields.time(name, unit)` require an explicit unit;
seconds/milliseconds select Time32 and microseconds/nanoseconds select Time64.
They accept the same native unit aliases as datatype parsing and reject
interval layouts.

```javascript
const { Record, fields } = require('yggdryl')

const Trade = Record.define('Trade', [
  fields.int64('id'),
  fields.utf8('symbol'),
  fields.decimal128('price', 18, 4),
])
const rows = [
  new Trade({ id: 1n, symbol: 'AAPL', price: 2246200n }),
  new Trade({ id: 2n, symbol: 'MSFT', price: 4187900n }),
]

const batch = Trade.intoArrowRecordBatch(rows)
const restored = [...Trade.fromArrowRecordBatch(batch)]

console.assert(restored[0].symbol === 'AAPL')
console.assert(restored[1].price === 4187900n)
```

Each defined class owns one shared native Record schema/name index. Apache
Arrow JS conversion is an explicit copied IPC boundary: it never serializes per
property access and does not claim zero-copy C Data interoperability. Use
`intoArrowRecordBatches` for bounded output, `fromArrowRecordBatchReader` for
batch-bounded input, and `intoArrowIPC`/`fromArrowIPC` for every physical layout
representable by the core `yggdryl::arrow` IPC runtime. The Apache Arrow
dependency loads only when an Arrow adapter is used.

`DataType.castArrowArray`, `Field.castArrowArray`, `DataType.castArrowBatch`,
and `Field.castArrowBatch` route copied Apache Arrow JS holders through the one
native `yggdryl::arrow` cast engine. Exact no-op casts preserve the original
Vector or RecordBatch identity; changed outputs follow `{safe: true}` by
default and never use a JavaScript conversion table.

Tabular cursor reads validate the declared source schema before any optional
target projection. Getters use a non-consuming snapshot. Overwrite, append,
indexed set, and positional upsert compute a complete validated replacement
before invoking one private atomic overwrite-all hook. `overwriteRecords`,
`appendRecords`, and `upsertRecords` provide the matching class-bound bulk
helpers and validate their options and source-to-backend cast plan before
consuming a one-shot row iterable. Dataset iterables likewise stop at their
first incompatible batch. Numeric selectors and matching table URLs with a
`#N` fragment address a batch; the exact table URL addresses the whole resource.
Tabular and dataset schema getters retain a stable canonical Arrow JS identity
and fingerprint-repair caller mutation from native state before reuse.

Run `npm run bench:record` or `npm run bench:tabular` to measure native Record
behavior and the copied-IPC read, cast, overwrite, indexed mutation, in-memory
table, and dataset paths separately. Set `YGGDRYL_BENCH_ITERATIONS` to change the
default sample count.

```javascript
const { Record, fields } = require('yggdryl')

const payload = fields.struct('payload', [
  fields.int64('count'),
  fields.utf8('note', { nullable: true }),
])
const value = payload.defaultJSValue()

console.assert(value instanceof Record)
console.assert(value.count === 0n && value.note === null)
console.assert(payload.defaultJSHint().constructor instanceof Function)
console.assert(fields.int32('id').defaultArrowScalar() === 0)
console.assert(
  fields.uint8('small').toSchemeCompat('spark').dataType.kind === 'int16',
)
```

Defaults are computed by the Rust core and projected through the same typed
path as Record values. Struct defaults remain schema-bound native Records;
runtime hints use a bounded native constructor category, are frozen and
cached, and do not materialize a default value or exact Field schema.
For a Struct Field, that metadata-free hint constructor is a logical layout
hint through the generic `Record` class, so nested metadata is not exposed by
the hint. The exact default Record still retains its complete outer Field and
uses its own generated class. Hint identity is wrapper-local, not a global
interning promise for equivalent schemas.
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
`fields.variant(name, fields, options)`. Both assign dense Arrow Union type IDs
in declaration order and retain the ordinary `union` kind; Record values use
the explicit `{ typeId, value }` shape. Iceberg/Parquet Variant and PostgreSQL
`json/jsonb` remain explicit adapter formats, not aliases for this tagged Arrow
layout.

```javascript
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const arrow = require('apache-arrow')
const { BatchReader, Field, IOBase, fields, iceberg } = require('yggdryl')

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
file.writeArrowBatchReader(BatchReader.from(rows))
console.assert(file.readArrowBatchReader().toTable().numRows === 2)

// An Iceberg table is a folder, and a folder is all it ever touches.
const table = iceberg.Table.create(path.join(root, 'trades'), schema, ['venue'])
table.append(rows)
console.assert(table.currentSnapshot.operation === 'append')
console.assert(table.dataFiles().length === 2)
```

`BatchReader` is the one record shape: a read returns one, a write consumes one,
and each batch crosses as its own Apache Arrow IPC stream, because Arrow JS has
no C Data consumer. `BatchReader.from` accepts another reader, an Arrow JS
`Table` or `RecordBatch`, an array of batches, or Arrow IPC bytes. The encoding
is never an argument: `recordOptions()` derives it from the handle's media type,
covering Arrow IPC and Apache Parquet.

Use the complete [JavaScript guide](https://platob.github.io/yggdryl/extensions/javascript/)
for schema and identifier APIs, the [Record and Arrow guide](https://platob.github.io/yggdryl/extensions/javascript/records/)
for typed rows and copied IPC interoperability, and the [codec guide](https://platob.github.io/yggdryl/extensions/javascript/codecs/)
for safe class registries, generic format inference, and streaming.
