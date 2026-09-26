# yggdryl-records in JavaScript

`const { IOBase, BatchReader, RecordOptions, TextOptions, iceberg } = require('yggdryl')` with `apache-arrow` for tables. Every record method takes a trailing `options?` - a `RecordOptions`, or a plain object of option properties (`{ select, filter, field, mergeBy, maxRowSize, commitRowSize, compression, rowheader }`) set on a copy of the handle's options. Batches cross as copied Arrow IPC, one self-contained batch at a time.

## Which encoding will this handle use?

The suffix or media type decides; `recordOptions()` answers that encoding's settings and throws for one the build does not implement; an absent resource reads as no batches.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { IOBase, MimeType, RecordOptions } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))

// The media type names the encoding: no format argument anywhere.
assert.equal(String(RecordOptions.from('trades.parquet').mimeType), 'application/vnd.apache.parquet')
assert.equal(RecordOptions.from('trades.arrows').maxRowGroupSize, null) // a Parquet-only setting
assert.equal(new IOBase(path.join(root, 'trades.avro')).recordOptions().blockCodec, 'deflate')

// Absent reads as empty; an unimplemented encoding is named, never guessed.
assert.equal([...new IOBase(path.join(root, 'absent.arrows')).readArrowReader()].length, 0)
const csv = IOBase.fromBytes()
csv.mediaType = MimeType.CSV
assert.throws(() => csv.recordOptions(), /text\/csv/)

fs.rmSync(root, { recursive: true, force: true })
```

## Write batches and stream them back

`readArrowReader()` answers a `BatchReader`: iterate it for Arrow JS batches, hand it to another writer, or `intoTable()` only when a whole table is wanted.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const arrow = require('apache-arrow')
const { IOBase } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
const source = new IOBase(path.join(root, 'trades.arrows'))
source.overwriteArrowTable(
  new arrow.Table({
    id: arrow.vectorFromArray([1n, 2n, 3n], new arrow.Int64()),
    venue: arrow.vectorFromArray(['XNAS', 'XNYS', null], new arrow.Utf8()),
  }),
)
assert.equal(source.readArrowField().name, 'row')
assert.deepEqual([source.rowSize, source.columnSize], [3, 2])

// Stream from one handle into another: nothing is collected on the way.
const target = new IOBase(path.join(root, 'trades.parquet'))
target.overwriteArrowReader(source.readArrowReader())

let rows = 0
for (const batch of target.readArrowReader()) rows += batch.numRows
assert.equal(rows, 3)

fs.rmSync(root, { recursive: true, force: true })
```

## Read only the columns and rows I need

Pass `select`, `filter`, a declared `field` and `maxRowSize` with the read: the medium projects and prunes, the field casts in the same pass, and the limit stops pulling.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const arrow = require('apache-arrow')
const { Field, IOBase, fields } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
const handle = new IOBase(path.join(root, 'trades.parquet'))
const ids = Array.from({ length: 10 }, (_, index) => BigInt(index))
handle.overwriteArrowTable(
  new arrow.Table({
    id: arrow.vectorFromArray(ids, new arrow.Int64()),
    venue: arrow.vectorFromArray(ids.map((id) => (id % 2n ? 'XNYS' : 'XNAS')), new arrow.Utf8()),
  }),
)

// Properties in a plain object, each set on a copy of the handle's options.
const picked = handle.readArrowReader({ select: ['id'], filter: 'id > 3', maxRowSize: 2 }).intoTable()
assert.deepEqual(picked.schema.fields.map((field) => field.name), ['id'])
assert.deepEqual([...picked.getChild('id')], [4n, 5n])

// A declared field projects and casts in one pass.
const narrow = fields.struct('row', [new Field('id', 'int32', false)], { nullable: false })
assert.equal(handle.readArrowReader(handle.recordOptions().withField(narrow)).intoTable().numCols, 1)

// The same sections as one options value, reusable across calls.
const options = handle.recordOptions().withPlan("select id where venue = 'XNYS' limit 3")
assert.deepEqual([...handle.readArrowReader(options).intoTable().getChild('id')], [1n, 3n, 5n])

fs.rmSync(root, { recursive: true, force: true })
```

## Write and read plain rows or class instances

`*Records` takes plain objects; `readRecords()` yields plain objects and `readRecords(Cls)` instances built from each row. Declare a `field` when writing: Arrow JS infers strings as dictionaries otherwise.

```javascript
const assert = require('node:assert/strict')
const { Field, IOBase, MimeType, fields } = require('yggdryl')

const schema = fields.struct('row', [new Field('id', 'int64', false), Field.from('venue: utf8')], {
  nullable: false,
})
const handle = IOBase.fromBytes()
handle.mediaType = MimeType.ARROW_STREAM
handle.overwriteRecords([{ id: 1n, venue: 'XNAS' }, { id: 2n, venue: 'XNYS' }], { field: schema })
handle.appendRecords([{ id: 3n, venue: 'XLON' }], { field: schema })
assert.equal(String(handle.readArrowField().fieldAt(1).dtype), 'utf8')

class Trade {
  constructor(row) {
    Object.assign(this, row)
  }
}
const trades = [...handle.readRecords(Trade, { filter: 'id >= 2' })]
assert.ok(trades.every((trade) => trade instanceof Trade))
assert.deepEqual(trades.map((trade) => trade.venue), ['XNYS', 'XLON'])
```

## Append, and upsert by key

Overwrite replaces, append keeps the stored rows, merge updates rows whose `mergeBy` key matches and appends the rest. Merge without a key throws.

```javascript
const assert = require('node:assert/strict')
const arrow = require('apache-arrow')
const { IOBase, MimeType } = require('yggdryl')

const rows = (ids, symbols) =>
  new arrow.Table({
    id: arrow.vectorFromArray(ids, new arrow.Int64()),
    symbol: arrow.vectorFromArray(symbols, new arrow.Utf8()),
  })

const handle = IOBase.fromBytes()
handle.mediaType = MimeType.ARROW_STREAM
handle.overwriteArrowTable(rows([1n, 2n], ['AAPL', 'MSFT']))
handle.appendArrowTable(rows([3n], ['NVDA']))
handle.mergeArrowTable(rows([2n, 9n], ['MSFT.O', 'AMD']), { mergeBy: ['id'] })

const stored = Object.fromEntries([...handle.readRecords()].map((row) => [row.id, row.symbol]))
assert.deepEqual(stored, { 1: 'AAPL', 2: 'MSFT.O', 3: 'NVDA', 9: 'AMD' })

assert.throws(() => handle.mergeArrowTable(rows([1n], ['X'])), /merge_by/)
```

## Choose the write mode at run time

`writeArrowReader|Table|Batch` and `writeRecords` take the mode as a string. `read_arrow`/`write_arrow` (the `SerieReader` doors, and the record door of JSON, YAML, TOML and XML handles) are Rust and Python only.

```javascript
const assert = require('node:assert/strict')
const arrow = require('apache-arrow')
const { IOBase, MimeType } = require('yggdryl')

const handle = IOBase.fromBytes()
handle.mediaType = MimeType.ARROW_STREAM
const table = new arrow.Table({ id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()) })
for (const mode of ['overwrite', 'append']) handle.writeArrowTable(table, mode)
assert.equal(handle.rowSize, 4)
```

## Bound memory on large writes

`commitRowSize: N` publishes every N rows (a committed prefix survives a later failure); unset commits once; `0` is refused before any input is pulled. `batchRowSize` bounds the batches a Parquet read yields.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const arrow = require('apache-arrow')
const { IOBase } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
const table = new arrow.Table({
  id: arrow.vectorFromArray(Array.from({ length: 10 }, (_, index) => BigInt(index)), new arrow.Int64()),
})
const handle = new IOBase(path.join(root, 'trades.parquet'))
handle.overwriteArrowTable(table, { commitRowSize: 4 })
assert.equal(handle.rowSize, 10)

const sizes = [...handle.readArrowReader({ batchRowSize: 4 })].map((batch) => batch.numRows)
assert.equal(sizes.reduce((a, b) => a + b, 0), 10)
assert.ok(Math.max(...sizes) <= 4)

assert.throws(() => handle.overwriteArrowTable(table, { commitRowSize: 0 }), /commit_row_size/)

fs.rmSync(root, { recursive: true, force: true })
```

## Parquet: compression, pruning, footer answers

Pages compress inside the file (`compression`, default `zstd(1)`); a read never names it. A `filter` skips row groups the footer rules out. An outer `.gz`/`.zst` on the name is refused.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const arrow = require('apache-arrow')
const { IOBase } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
const ids = Array.from({ length: 1_000 }, (_, index) => BigInt(index))
const table = new arrow.Table({
  id: arrow.vectorFromArray(ids, new arrow.Int64()),
  symbol: arrow.vectorFromArray(ids.map(() => 'AAPL'), new arrow.Utf8()),
})

const handle = new IOBase(path.join(root, 'trades.parquet'))
handle.overwriteArrowTable(table, { compression: 'snappy', maxRowGroupSize: 250 })

// rowSize and the statistics come from the footer, never from decoding rows.
assert.equal(handle.rowSize, 1_000)
assert.equal(handle.readParquetStatistics().row_groups.length, 4)
assert.equal(handle.readArrowReader({ filter: 'id >= 900' }).intoTable().numRows, 100)

const coded = new IOBase(path.join(root, 'trades.parquet.gz'))
assert.throws(() => coded.overwriteArrowTable(table), /parquet compresses/)

fs.rmSync(root, { recursive: true, force: true })
```

## Avro: a container file, or bytes with a reader schema

A `.avro` handle is a record medium (`blockCodec`: `null`, `deflate`, `snappy`, `zstandard`). The `avro` namespace is the scalar codec over bytes; a reader schema resolves renames, promotions and defaults.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const arrow = require('apache-arrow')
const { IOBase, avro } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
const handle = new IOBase(path.join(root, 'trades.avro'))
handle.overwriteArrowTable(
  new arrow.Table({
    symbol: arrow.vectorFromArray(['AAPL'], new arrow.Utf8()),
    qty: arrow.vectorFromArray([100n], new arrow.Int64()),
  }),
  { blockCodec: 'zstandard' },
)
assert.deepEqual([...handle.readRecords({ select: ['qty'] })], [{ qty: 100n }])

const writer = {
  type: 'record',
  name: 'trade',
  fields: [{ name: 'symbol', type: 'string' }, { name: 'qty', type: 'int' }],
}
const reader = new avro.Schema({
  type: 'record',
  name: 'trade',
  fields: [
    { name: 'quantity', aliases: ['qty'], type: 'long' },
    { name: 'note', type: 'string', default: 'none' },
  ],
})
const encoded = avro.dumps([{ symbol: 'AAPL', qty: 100 }], writer)
assert.deepEqual(avro.loads(encoded, { readerSchema: reader }).rows, [{ note: 'none', quantity: 100 }])

fs.rmSync(root, { recursive: true, force: true })
```

## Read a log file as typed rows

A `.log`/`.txt` handle reads one record per line (or per framed chain with `framing`): the sixteen event columns, `body`, then one column per named `rowheader` capture, typed by `autotype` (on by default).

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { IOBase, TextOptions } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
const source = path.join(root, 'app.log')
fs.writeFileSync(source, '[INFO] id=7 first\r\n detail A\r[WARN] id=9 second\n detail B')

const options = new TextOptions()
options.startRownum = 1n
options.rowheader = '^\\[(?<level>[A-Z]+)\\] id=(?<id>\\d+) '
options.framing = true

const rows = [...new IOBase(source).intoText(options).readRecords()]
assert.deepEqual(rows.map((row) => row.id), [7n, 9n])
assert.deepEqual(rows.map((row) => row.body), ['first\n detail A', 'second\n detail B'])
assert.deepEqual(rows.map((row) => row.seqnum), [1n, 3n])

// One-off: the same property in the options object, no TextOptions value.
const levels = new IOBase(source).readArrowReader({ rowheader: '^\\[(?<level>[A-Z]+)\\] ' })
assert.deepEqual(levels.intoTable().schema.fields.slice(-2).map((field) => field.name), ['body', 'level'])

fs.rmSync(root, { recursive: true, force: true })
```

## Partitioned folders: route on write, prune on read

Addressing a folder writes each row to its `column=value` leaf (the path carries the partition columns, the leaf does not) and reads them back typed. A `filter` equality prunes leaves by path before anything is decoded.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const arrow = require('apache-arrow')
const { Field, IOBase, MimeType, RecordOptions, fields } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
fs.mkdirSync(path.join(root, 'year=2024', 'month=01'), { recursive: true })
const schema = fields.struct(
  'row',
  [Field.from('price: int64'), Field.from('year: int32'), Field.from('month: utf8')],
  { nullable: false },
)
const lake = new IOBase(root)
const options = RecordOptions.forMimeType(MimeType.ARROW_STREAM).withField(schema)
lake.overwriteArrowTable(
  new arrow.Table({
    price: arrow.vectorFromArray([10n, 20n], new arrow.Int64()),
    year: arrow.vectorFromArray([2024, 2024], new arrow.Int32()),
    month: arrow.vectorFromArray(['01', '01'], new arrow.Utf8()),
  }),
  options,
)

const leaf = lake.joinpath('year=2024', 'month=01', 'part-0.arrows')
assert.equal(leaf.readArrowField().dtype.length, 1) // only `price` is stored
assert.equal(lake.readArrowReader(options).intoTable().numCols, 3)

const pruned = options.withFilter("year = 2024 and month = '01'")
assert.deepEqual(pruned.partitionPairs(), [['year', '2024'], ['month', '01']])
assert.equal(lake.readArrowReader(pruned).intoTable().numRows, 2)
assert.equal([...lake.childrenWhere({ year: '2024' })].length, 1)

fs.rmSync(root, { recursive: true, force: true })
```

## Derive a partition column from another column

Not bound in JavaScript: the `PARTITION:` view's `apply_arrow_batch` is Rust and Python only. Declare and fill the derived column there, or write it as an ordinary column.

## Iceberg: create, append, upsert, scan

A table is a folder; no catalog is required. Scans are `BatchReader`s planned from metadata; `merge` keys are the identity partition columns plus `mergeBy`.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const arrow = require('apache-arrow')
const { Field, IOBase, fields, iceberg } = require('yggdryl')

const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-')), 'trades')
const schema = fields.struct(
  'row',
  [new Field('id', 'int64', false), Field.from('venue: utf8'), Field.from('px: float64')],
  { nullable: false },
)
const rows = (ids, venues, prices) =>
  new arrow.Table({
    id: arrow.vectorFromArray(ids, new arrow.Int64()),
    venue: arrow.vectorFromArray(venues, new arrow.Utf8()),
    px: arrow.vectorFromArray(prices, new arrow.Float64()),
  })

const table = iceberg.Table.create(root, schema, ['venue'])
assert.equal(table.currentSnapshot, null)
table.append(rows([1n, 2n, 3n], ['XNAS', 'XNYS', 'XNAS'], [1, 2, 3]))
const first = table.currentSnapshot.snapshotId
table.merge(rows([3n, 4n], ['XNAS', 'XNAS'], [30, 4]), ['id'])

assert.equal(table.scan().intoTable().numRows, 4)
assert.deepEqual([...table.scanMatching('px > 2.5').intoTable().getChild('px')], [30, 4])
assert.equal(table.planMatching("venue = 'XNYS'").tasks, 1)
assert.equal(table.scanAt(first).intoTable().numRows, 3) // time travel

// The folder is also an ordinary record handle: filter and select push down.
const pushed = new IOBase(root).readArrowReader({ select: ['id'], filter: "venue = 'XNYS'" })
assert.equal(pushed.intoTable().numRows, 1)

fs.rmSync(path.dirname(root), { recursive: true, force: true })
```

## Evolve an Iceberg schema

`updateSchema()` records a chain and `commit()` writes one new schema and returns its id; field IDs are kept and never reused. `evolveSchema(field)` replaces the schema whole.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const arrow = require('apache-arrow')
const { Field, fields, iceberg } = require('yggdryl')

const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-')), 'trades')
const table = iceberg.Table.create(root, fields.struct('row', [new Field('id', 'int32', false)], { nullable: false }))
table.append(new arrow.Table({ id: arrow.vectorFromArray([1], new arrow.Int32()) }))

const schemaId = table.updateSchema().addColumn('', Field.from('note: utf8')).updateType('id', 'int64').commit()
assert.equal(schemaId, 1)
assert.deepEqual([...table.schema.dtype].map((child) => child.name), ['id', 'note'])
assert.deepEqual([...table.scan().intoTable().getChild('note')], [null])

fs.rmSync(path.dirname(root), { recursive: true, force: true })
```

## Hand the file to polars or a pyarrow dataset lazily

Python only (`scan_polars`, `scan_arrow`). In JavaScript, iterate `readArrowReader()` with the pushdown properties above.

## Run a SQL-like write or read plan

`Plan` spells `insert into`, `insert overwrite`, `upsert into ... by (...)`, `delete from ... where`, and reads with `select ... from ... where ... limit ... offset`; `execute()` pushes the read sections into the source. Grammar: `yggdryl-expressions`.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { pathToFileURL } = require('node:url')
const arrow = require('apache-arrow')
const { Plan } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
const url = pathToFileURL(path.join(root, 'trades.arrows')).href
new Plan(`create '${url}' (id int64 not null, name utf8)`).execute().intoTable()
new Plan(`insert into '${url}'`).applyArrowBatch(
  new arrow.Table({
    id: arrow.vectorFromArray([1n, 2n, 3n], new arrow.Int64()),
    name: arrow.vectorFromArray(['a', 'b', 'c'], new arrow.Utf8()),
  }).batches[0],
)

const read = new Plan(`select name from '${url}' where id > 1 order by id desc limit 1 offset 1`)
assert.deepEqual([...read.execute().intoTable().getChild('name')], ['b'])

fs.rmSync(root, { recursive: true, force: true })
```

## Gotchas in JavaScript

- Arrow JS interop is copied IPC: cross in whole batches or tables, never row by row; `intoTable()` drains the reader.
- `int64` columns come back as `bigint`; build Arrow JS `Int64` vectors from `bigint` (`1n`).
- Plain-object rows infer strings as `dictionary(int32,utf8)`; Avro cannot store that. Pass `{ field }` on the write.
- A `RecordOptions` `with*` call returns a new value; setters (`options.filter = ...`) mutate that one object.
- `RecordOptions` has no offset: a plan's `offset` given through `withPlan` is dropped. Use `Plan.applyArrowReader(reader)` or `Plan.execute()`.
- No `readArrow`/`writeArrow` and no `scanPolars`: structured-text rows go through the codecs in `yggdryl-documents`.
