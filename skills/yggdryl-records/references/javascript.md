# yggdryl-records in JavaScript

`const { IOBase, BatchReader, RecordOptions, TextOptions, iceberg } = require('yggdryl')` with `apache-arrow` for tables. Every record method takes a trailing `options?` - a `RecordOptions`, or a plain object of option properties (`{ select, filter, field, mergeBy, maxRowSize, rowOffset, commitBatchNum, numThreads, compression, rowheader }`) set on a copy of the handle's options. Batches cross as copied Arrow IPC, one self-contained batch at a time.

## Which encoding will this handle use?

The suffix or media type decides; `recordOptions()` answers that encoding's settings and throws for a type no medium claims (naming the claimed ones and the crate to install); an absent resource reads as no batches.

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
const orc = IOBase.fromBytes()
orc.mediaType = MimeType.ORC
assert.throws(() => orc.recordOptions(), /application\/vnd\.apache\.orc/)

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
assert.deepEqual([source.rowSize(), source.columnSize()], [3, 2])

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

// rowOffset skips leading rows before maxRowSize counts.
const page = handle.readArrowReader({ select: ['id'], rowOffset: 3, maxRowSize: 2 }).intoTable()
assert.deepEqual([...page.getChild('id')], [3n, 4n])

// A declared field projects and casts in one pass.
const narrow = fields.struct('row', [new Field('id', 'int32', false)], { nullable: false })
assert.equal(handle.readArrowReader(handle.recordOptions().withField(narrow)).intoTable().numCols, 1)

// The same sections as one options value, reusable across calls.
const options = handle.recordOptions().withPlan("select id where venue = 'XNYS' limit 3")
assert.deepEqual([...handle.readArrowReader(options).intoTable().getChild('id')], [1n, 3n, 5n])

fs.rmSync(root, { recursive: true, force: true })
```

## Refuse a value the declared field cannot convert

A nullable declared column takes a value it cannot convert as null while `safe` holds, and `safe` is the default; `{ safe: false }` (or a `not null` column) refuses it by path.

```javascript
const assert = require('node:assert/strict')
const arrow = require('apache-arrow')
const { Field, IOBase, MimeType } = require('yggdryl')

const handle = IOBase.fromBytes()
handle.mediaType = MimeType.PARQUET
handle.overwriteArrowTable(new arrow.Table({ v: arrow.vectorFromArray(['1', 'x'], new arrow.Utf8()) }))
const field = Field.from('row: struct<v: int32> not null')

// Default: the unconvertible 'x' becomes null without a word.
const read = handle.readArrowReader({ field }).intoTable()
assert.deepEqual([...read.getChild('v')], [1, null])

// safe: false names the column and the value instead.
assert.throws(() => handle.readArrowReader({ field, safe: false }).intoTable(), /\$\.v/)
```

## Write and read plain rows or class instances

`*StreamSerie` takes plain objects; `readRecords()` yields plain objects and `readRecords(Cls)` instances built from each row. Declare a `field` when writing: Arrow JS infers strings as `dictionary(int32,utf8)` otherwise.

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

// number and bigint mix in one call - an integral number beside bigint rows
// reads as bigint - and only a fraction beside them is refused, by column
// and value.
const mixed = IOBase.fromBytes()
mixed.mediaType = MimeType.ARROW_STREAM
mixed.overwriteRecords([{ id: 4, venue: 'XPAR' }, { id: 5n, venue: 'XPAR' }], { field: schema })
assert.deepEqual([...mixed.readArrowReader().intoTable().getChild('id')], [4n, 5n])
assert.throws(
  () => mixed.appendRecords([{ id: 1.5, venue: 'XPAR' }, { id: 6n, venue: 'XPAR' }], { field: schema }),
  (error) => error instanceof TypeError && /"id"/.test(error.message),
)

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

Overwrite replaces, append keeps the stored rows, merge updates rows whose `mergeBy` key matches and appends the rest, replacing a stored row only where the last arrival for its key differs from it - a merge changing nothing leaves the file unwritten. Merge without a key throws; `mergeBy` takes no boolean.

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

`writeArrowReader|Table|Batch` and `writeRecords` take the mode as a string, and so does `writeSerie(value, mode?)`, whose `overwriteSerie`/`appendSerie`/`mergeSerie` name it: `value` is a `Serie`, a `ChunkedSerie`, a `StreamChunkedSerie` (consumed) or anything `BatchReader.from` accepts, and with `readSerie()` - a generic `Serie` - they are also the record door of JSON, JSON Lines, YAML, TOML and XML handles. Absent options are the handle's own.

```javascript
const assert = require('node:assert/strict')
const arrow = require('apache-arrow')
const { ChunkedSerie, IOBase, MimeType, Serie, StreamChunkedSerie } = require('yggdryl')

const handle = IOBase.fromBytes()
handle.mediaType = MimeType.ARROW_STREAM
const table = new arrow.Table({ id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()) })
for (const mode of ['overwrite', 'append']) handle.writeArrowTable(table, mode)
assert.equal(handle.rowSize(), 4)

// The Serie doors take a held column, held chunks, a stream or any batch source.
const rows = Serie.fromArrowBatch(table)
handle.writeSerie(rows, 'append')
handle.appendSerie(ChunkedSerie.fromArrowBatch(table))
assert.equal(handle.rowSize(), 8)
const read = handle.readSerie()
assert.ok(read instanceof Serie)
assert.equal([...read.intoChunkedStream()].reduce((total, records) => total + records.length, 0), 8)
```

## Bound memory on large writes

`commitBatchNum: N` publishes every N whole batches, then the remainder (a committed prefix survives a later failure); a cadence never cuts a batch, and records are cut into batches by `batchRowSize`. Unset is the destination's own cadence - a file, a folder and an Iceberg table commit once, the table holding every partition's rows under the process spill bound until the source ends; `0` is refused before any input is pulled. `numThreads: n` is how many partition groups an Iceberg commit writes at once, `0` refused naming `$.num_threads`. `batchRowSize` bounds the batches a Parquet read yields.

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
// Three batches at two batches a commit: rows 0-7 publish, then rows 8-9.
const batches = new arrow.Table([0, 4, 8].flatMap((start) => table.slice(start, start + 4).batches))
handle.overwriteArrowTable(batches, { commitBatchNum: 2 })
assert.equal(handle.rowSize(), 10)

const sizes = [...handle.readArrowReader({ batchRowSize: 4 })].map((batch) => batch.numRows)
assert.equal(sizes.reduce((a, b) => a + b, 0), 10)
assert.ok(Math.max(...sizes) <= 4)

assert.throws(() => handle.overwriteArrowTable(table, { commitBatchNum: 0 }), /commit_batch_num/)

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
assert.equal(handle.rowSize(), 1_000)
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

## Excel: one worksheet as records, the workbook as cells

A `.xlsx` handle is a record medium over one worksheet - `sheet`, `header` and `range` pick which cells - and `Workbook` is the same package cell by cell.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const arrow = require('apache-arrow')
const { IOBase, Serie, Sheet, Workbook } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
const file = path.join(root, 'trades.xlsx')
const table = new arrow.Table({
  id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
  symbol: arrow.vectorFromArray(['AAPL', null], new arrow.Utf8()),
})

// One worksheet as records, under the declared field.
const handle = new IOBase(file)
handle.overwriteArrowTable(table, { sheet: 'Trades' })
const field = Serie.fromArrowBatch(table).field
assert.deepEqual([...handle.readArrowReader({ field, sheet: 'Trades' }).intoTable().getChild('id')], [1n, 2n])
// Inferred, a number column is the float64 the file stores.
assert.deepEqual([...handle.readArrowReader({ sheet: 'Trades' }).intoTable().getChild('id')], [1, 2])

// The workbook: any cell by its A1 reference, a sheet as a Serie and back.
const workbook = Workbook.open(file)
const sheet = workbook.sheet('Trades')
assert.equal(sheet.cell('B2').value.asJs(), 'AAPL')
sheet.setCell('B3', 'MSFT')
assert.equal(sheet.intoSerie(field).asJs().length, 2)
workbook.insertSheet(Sheet.fromSerie('Copy', table))
workbook.writeInto(file)
assert.deepEqual(Workbook.open(file).sheetNames, ['Trades', 'Copy'])
fs.rmSync(root, { recursive: true, force: true })
```

## Read a log file as typed rows

A `.log`/`.txt` handle reads one record per line (or per framed chain with `framing`): the fifteen event columns, `body`, then one column per named `rowheader` capture, typed by `autotype` (on by default).

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

## CSV and TSV: the dialect in the options object

A `.csv` handle reads RFC 4180 records under its header and a sample of the rows, or under the declared `field`; a `.tsv` name is the same medium under a tab. The dialect is option properties (`separator`, `quote`, `escape`, `comment`, `header`, `nullValues`, `trim`, `inferRowSize`), each also a key of the options object on any read or write; compression is the name's (`trades.csv.gz`).

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { Field, IOBase, RecordOptions, fields } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
const field = fields.struct('trade', [new Field('id', 'int64', false), Field.from('symbol: utf8')], {
  nullable: false,
})
const rows = [{ id: 1n, symbol: 'AAPL' }, { id: 2n, symbol: null }, { id: 3n, symbol: '' }]

// The name says CSV and gzip; the declared field is the contract every cell crosses.
const handle = new IOBase(path.join(root, 'trades.csv.gz'))
assert.equal(String(handle.recordOptions().mimeType), 'text/csv')
handle.overwriteRecords(rows, { field })
// A null is the empty cell; the empty text is quoted, so the two read back apart.
assert.deepEqual([...handle.readRecords({ field })], rows)

// Undeclared, the header names the columns and the sample types them, every one nullable.
assert.deepEqual(Array.from(handle.readArrowField().dtype, (child) => child.name), ['id', 'symbol'])
assert.deepEqual([handle.rowSize(), handle.columnSize()], [3, 2])

// A `;` document another writer saved: the separator is a property of the read.
fs.writeFileSync(path.join(root, 'eu.csv'), 'id;symbol\n1;AAPL\n2;\n')
assert.deepEqual([...new IOBase(path.join(root, 'eu.csv')).readRecords({ separator: ';' })], rows.slice(0, 2))
// Under the default dialect the same header is one column.
assert.equal(new IOBase(path.join(root, 'eu.csv')).readArrowField().dtype.length, 1)

// The dialect on an options value: a byte role is one character, null clears one.
const options = RecordOptions.from('trades.csv').withSeparator('|').withQuote(null).withNullValues(['NA', ''])
assert.deepEqual([options.separator, options.quote, options.nullValues], ['|', null, ['NA', '']])
assert.equal(RecordOptions.from('trades.tsv').separator, '\t')
assert.equal(RecordOptions.from('trades.parquet').separator, null) // a CSV-only setting

// A record with the wrong number of cells is refused by row, never widened.
fs.writeFileSync(path.join(root, 'ragged.csv'), 'a,b\n1,2\n3\n')
assert.throws(() => [...new IOBase(path.join(root, 'ragged.csv')).readRecords()], /expected 2 cells, got 1 in row 3/)

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

`PARTITION:by` declares it - a bare column an identity partition, a term a derived one (`years(event)`, `truncate(name, 4) as prefix`) - and `withPartitionBy` marks the identity columns and adds each derived entry as a marked column carrying its term as `TRANSFORM:` metadata. The field views have no `applyArrowBatch`: the root's `Selector` computes the derived column.

```javascript
const assert = require('node:assert/strict')
const arrow = require('apache-arrow')
const { DataType, Field, Selector } = require('yggdryl')

const root = new Field('row', DataType.fromFields([new Field('event', 'date32', false)]), false)
  .withPartitionBy(['year(event) as year'])
assert.deepEqual(root.partitionFieldNames(), ['year'])
assert.deepEqual(root.partitionBy(), ['year(event) as year'])

const batch = new arrow.Table({
  event: arrow.vectorFromArray([new Date('2024-01-01'), new Date('2025-01-01')], new arrow.DateDay()),
}).batches[0]
const filled = Selector.fromField(root).applyArrowBatch(batch)
assert.deepEqual(filled.schema.fields.map((field) => field.name), ['event', 'year'])
assert.deepEqual([...filled.getChild('year')], [2024, 2025])
```

## Iceberg: create, append, upsert, scan

A table is a folder; no catalog is required. Scans are `BatchReader`s planned from metadata; `merge` keys are the identity partition columns plus `mergeBy`, else - `mergeBy` left out or `null`, JavaScript taking no boolean - the columns the schema's `identifier-field-ids` names. On a table stating `identifier-field-ids`, `append` writes only the rows whose key neither the table nor an earlier row of the write holds, and a merge or an append that changes nothing commits no snapshot.

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

const table = iceberg.IcebergTable.create(root, schema, ['venue'])
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
const table = iceberg.IcebergTable.create(root, fields.struct('row', [new Field('id', 'int32', false)], { nullable: false }))
table.append(new arrow.Table({ id: arrow.vectorFromArray([1], new arrow.Int32()) }))

const schemaId = table.updateSchema().addColumn('', Field.from('note: utf8')).updateType('id', 'int64').commit()
assert.equal(schemaId, 1)
assert.deepEqual([...table.schema.dtype].map((child) => child.name), ['id', 'note'])
assert.deepEqual([...table.scan().intoTable().getChild('note')], [null])

fs.rmSync(path.dirname(root), { recursive: true, force: true })
```

## Write a pandas or polars frame to a file, and read one back

Python only (`overwrite_pandas_frame`, `read_polars_frame`, ...). In JavaScript, write an Arrow JS table with `overwriteArrowTable` and read it back with `readArrowReader().intoTable()`.

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
- `int64` columns come back as `bigint`; build Arrow JS `Int64` vectors from `bigint` (`1n`). `*StreamSerie` unifies `number` and `bigint` rows of one column into one type: `[{ id: 1 }, { id: 2n }]` is one `int64` column, an integral `number` read as `bigint`. Only a fraction beside `bigint` rows in that column is refused - `TypeError`, naming the column and the value. A later batch reads a `number` as `bigint` under an established `int64` column, and a `bigint` as `number` under a `number` column when it is a safe integer, else the same named `TypeError`.
- A declared nullable column reads a value it cannot convert as null under the default `safe`; pass `{ safe: false }` to have it refused.
- Plain-object rows infer strings as `dictionary(int32,utf8)`; Avro stores them as the plain values. Pass `{ field }` on the write to state the column instead.
- A `RecordOptions` `with*` call returns a new value; setters (`options.filter = ...`) mutate that one object.
- A CSV byte role (`separator`, `quote`, `escape`, `comment`) is a one-character string, `null` clearing an optional one; the role itself (ASCII, no line break, no byte another role holds) is judged by the core, and a CSV property on another encoding's options reads `null` and throws when set.
- A plan's `offset` given through `withPlan` is the options' `rowOffset`; a merge with one is refused.
- No `scanPolars`. Structured-text rows go through `readSerie`/`overwriteSerie` - a document takes `overwrite` alone - or the codecs in `yggdryl-documents`.
