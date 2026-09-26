# yggdryl-arrow in JavaScript

`const { Serie, ChunkedSerie, SerieReader, ArrowCastPlan, BatchReader, Field,
fields } = require('yggdryl')` beside `require('apache-arrow')`. Every Arrow
door is copied IPC - one self-contained IPC stream per vector, batch or table -
so cross whole tables or readers, never a row at a time. Cast options are
`{ safe, representation }`; an absent key is skipped, an unknown key refused.

## Build a column from JavaScript values

`Serie.fromScalars` sends every row through the field's value contract once;
`fromDefault` repeats the field's canonical default. `int64` rows are
`bigint` going in.

```javascript
const assert = require('node:assert/strict')
const { Field, Serie, fields } = require('yggdryl')

const price = fields.int64('price', { nullable: false })
const serie = Serie.fromScalars(price, [125n, 126n, 127n])
assert.equal(serie.length, 3)
assert.ok(serie.field.equals(price))
assert.deepEqual(serie.asJs(), [125, 126, 127])

// A required field has no room for null: the whole column is refused by path.
assert.throws(() => Serie.fromScalars(price, [null]), /\$\.price/)

// Defaults: a required field repeats its present default, a nullable one null.
assert.deepEqual(Serie.fromDefault(price, 2).asJs(), [0, 0])
assert.deepEqual(Serie.fromDefault(Field.from('symbol: utf8'), 2).asJs(), [null, null])
assert.equal(Serie.empty(price).length, 0)
```

## Land an Arrow JS vector

`Serie.fromArrowArray(vector, field?, options?)` lands the vector once under
its own layout and casts it once into `field`, however many `Data` it holds.
With no field it is the column of its own layout.

```javascript
const assert = require('node:assert/strict')
const arrow = require('apache-arrow')
const { Serie, fields } = require('yggdryl')

const price = fields.int64('price', { nullable: false })
const vector = arrow.vectorFromArray([125n, 126n, 127n], new arrow.Int64())
const held = Serie.fromArrowArray(vector, price)
assert.ok(held.equals(Serie.fromScalars(price, [125n, 126n, 127n])))

// No field: the column of its own layout.
assert.equal(Serie.fromArrowArray(vector).field.dtype.toString(), 'int64')

// Another layout is cast once, on the way in.
const text = arrow.vectorFromArray(['12', '1234'], new arrow.Utf8())
assert.deepEqual(Serie.fromArrowArray(text, price).asJs(), [12, 1234])

// Back out: a new Arrow JS vector (a copy).
assert.deepEqual([...held.intoArrowArray()], [125n, 126n, 127n])
```

## Land a record batch or table as a record column

`Serie.fromArrowBatch(batchOrTable, root?)` lands the rows under a non-null
struct root: children matched by name (ASCII case-insensitive), in the
root's order, extra columns dropped, missing nullable ones all-null.

```javascript
const assert = require('node:assert/strict')
const arrow = require('apache-arrow')
const { Field, Serie } = require('yggdryl')

const root = Field.from('trade: struct<id: int64 not null, symbol: utf8, venue: utf8> not null')
const table = new arrow.Table({
  SYMBOL: arrow.vectorFromArray(['ACME'], new arrow.Utf8()),
  id: arrow.vectorFromArray([7], new arrow.Int32()),
  extra: arrow.vectorFromArray([1.5], new arrow.Float64()),
})

const trades = Serie.fromArrowBatch(table, root)
assert.deepEqual(trades.names, ['id', 'symbol', 'venue'])
assert.deepEqual(trades.child('id').asJs(), [7])
assert.deepEqual(trades.child('venue').asJs(), [null])

const batch = trades.intoArrowBatch()
assert.deepEqual(batch.schema.fields.map((field) => field.name), ['id', 'symbol', 'venue'])

// With no root the table is the record `row` of its own schema.
assert.equal(Serie.fromArrowBatch(table).field.name, 'row')
```

## Turn Arrow JS values into a native stream

JavaScript has no C Data consumer, so there is no `from_` ladder:
`BatchReader.from` converts a `Table`, a `RecordBatch`, an array of batches or
IPC bytes into the native `BatchReader` every reader door takes.
`intoTable()` and `intoIpc()` drain it.

```javascript
const assert = require('node:assert/strict')
const arrow = require('apache-arrow')
const { BatchReader, Serie } = require('yggdryl')

const table = new arrow.Table({ id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()) })

const reader = BatchReader.from(table)
assert.equal(reader.field.name, 'row')
assert.equal(reader.consumed, false)
const bytes = reader.intoIpc()
assert.equal(reader.consumed, true)

// IPC bytes are a source too, and a drained reader becomes a Table again.
assert.equal(BatchReader.fromIpc(bytes).intoTable().numRows, 2)
assert.equal([...BatchReader.from(table)].length, 1)

// Reader doors take only a native BatchReader, and consume it.
const drained = Serie.fromArrowReader(BatchReader.from(table))
assert.deepEqual(drained.child('id').asJs(), [1, 2])
assert.throws(() => Serie.fromArrowReader(table))
```

## Stream a reader under one plan

`SerieReader.fromArrowReader(reader, root?)` compiles one plan from the
stream's schema before a batch is pulled; each pulled batch is one record
`Serie`. A bad batch fails at its pull and the reader is fused after it.
`intoArrowReader()` is the transport face, never landed.

```javascript
const assert = require('node:assert/strict')
const arrow = require('apache-arrow')
const { BatchReader, Field, SerieReader } = require('yggdryl')

const root = Field.from('row: struct<id: int64, symbol: utf8 not null> not null')
const batch = (id, symbol) =>
  new arrow.Table({
    id: arrow.vectorFromArray([id], new arrow.Int32()),
    symbol: arrow.vectorFromArray([symbol], new arrow.Utf8()),
  }).batches
const source = () => new arrow.Table([...batch(1, 'A'), ...batch(2, 'B'), ...batch(3, null)])

const series = SerieReader.fromArrowReader(BatchReader.from(source()), root)
assert.ok(series.field.equals(root))
const pulled = series[Symbol.iterator]()
assert.deepEqual(pulled.next().value.child('id').asJs(), [1])
assert.deepEqual(pulled.next().value.child('symbol').asJs(), ['B'])
assert.throws(() => pulled.next(), /required Arrow field \$\.symbol holds 1 null values/)
assert.equal(pulled.next().done, true)

// Transport: a native BatchReader under the root's schema, read once.
const good = new arrow.Table([...batch(1, 'A'), ...batch(2, 'B')])
const reader = SerieReader.fromArrowReader(BatchReader.from(good), root).intoArrowReader()
assert.ok(reader instanceof BatchReader)
assert.equal(reader.intoTable().numRows, 2)
```

## Compile a cast once and apply it to many batches

`ArrowCastPlan.compile(source, target, options?)` does every schema-dependent
decision once; an Arrow JS schema, table or batch is the record `row`.
`apply` takes a `Serie` or a `ChunkedSerie`.

```javascript
const assert = require('node:assert/strict')
const arrow = require('apache-arrow')
const { ArrowCastPlan, ChunkedSerie, Field, Serie } = require('yggdryl')

const root = Field.from('row: struct<id: int64 not null> not null')
const table = new arrow.Table({ id: arrow.vectorFromArray([1, 2], new arrow.Int32()) })

const plan = ArrowCastPlan.compile(table.schema, root)
plan.preflight() // the whole recursion over no rows
assert.equal(plan.source.name, 'row')
assert.ok(plan.target.equals(root))
assert.equal(plan.isIdentity, false)
assert.deepEqual(plan.options, { safe: true, representation: 'value' })

for (const batch of table.batches) {
  assert.deepEqual(plan.apply(Serie.fromArrowBatch(batch)).child('id').asJs(), [1, 2])
}
const chunked = plan.apply(ChunkedSerie.fromArrowBatch(table))
assert.ok(chunked instanceof ChunkedSerie)

// An identity plan hands the column back as it is.
assert.equal(ArrowCastPlan.compile(root, root).isIdentity, true)

// A missing required column is refused when the plan is compiled.
const other = new arrow.Table({ other: arrow.vectorFromArray([1], new arrow.Int32()) })
assert.throws(
  () => ArrowCastPlan.compile(other.schema, root),
  /required Arrow field \$\.id is missing from the source/,
)
```

## Cast a column in hand

`serie.cast(field, options?)` is one plan for one column; a column already
under the target is itself. A `DataType` target is the required field named
`value`.

```javascript
const assert = require('node:assert/strict')
const { DataType, Serie, fields } = require('yggdryl')

const ids = Serie.fromScalars(fields.int32('id'), [1, null])
assert.deepEqual(ids.cast(fields.int64('id')).asJs(), [1, null])
assert.ok(ids.cast(ids.field).equals(ids))
assert.throws(
  () => ids.cast(DataType.from('int64')),
  /required Arrow field \$\.value holds 1 null values/,
)

// A schema-free run has no buffers to cast: type its rows instead.
assert.throws(() => new Serie([1n, 2n]).cast(fields.int64('id')), /run/)
assert.deepEqual(Serie.fromScalars(fields.int64('id'), [1n, 2n]).asJs(), [1, 2])
```

## Decide what a bad or missing value becomes

Nullability of the target field decides absence; `safe` decides only whether
a present value that fails to convert becomes null, and only where the column
may hold null. An empty text cell is null before `safe` is asked.

```javascript
const assert = require('node:assert/strict')
const arrow = require('apache-arrow')
const { Serie, fields } = require('yggdryl')

const broken = arrow.vectorFromArray(['1', 'not a number', ''], new arrow.Utf8())
const nullable = fields.int64('n')
const required = fields.int64('n', { nullable: false })

// Nullable + safe (default): the failure and the empty cell are null.
assert.deepEqual(Serie.fromArrowArray(broken, nullable).asJs(), [1, null, null])

// Nullable + safe: false: the failed conversion is refused by value.
assert.throws(() => Serie.fromArrowArray(broken, nullable, { safe: false }), /not a number/)

// Required: refused whatever safe says; never filled with a default.
for (const options of [{ safe: true }, { safe: false }]) {
  assert.throws(() => Serie.fromArrowArray(broken, required, options), /not a number/)
}

// An empty cell into a required column is a null, refused by path.
const empty = arrow.vectorFromArray([''], new arrow.Utf8())
assert.throws(
  () => Serie.fromArrowArray(empty, required, { safe: false }),
  /required Arrow field \$\.n holds 1 null values/,
)
```

## Reinterpret bits between same-width types

`representation: 'bits'` carries the bytes between two layouts of one byte
width; other pairs convert as usual.

```javascript
const assert = require('node:assert/strict')
const arrow = require('apache-arrow')
const { Serie, fields } = require('yggdryl')

const bits = { representation: 'bits' }
const source = arrow.vectorFromArray([0n, 2n ** 64n - 1n], new arrow.Uint64())

const signed = Serie.fromArrowArray(source, fields.int64('digest'), bits)
assert.deepEqual([...signed.intoArrowArray()], [0n, -1n])

const raw = Serie.fromArrowArray(source, fields.fixedSizeBinary('digest', 8), bits)
assert.deepEqual([...raw.intoArrowArray().get(1)], new Array(8).fill(255))
assert.deepEqual([...raw.cast(fields.uint64('digest'), bits).intoArrowArray()], [...source])

// Four bytes are not eight: an ordinary widening.
const narrow = arrow.vectorFromArray([7], new arrow.Uint32())
assert.deepEqual(Serie.fromArrowArray(narrow, fields.int64('id'), bits).asJs(), [7])
```

## Read values fast

JavaScript binds no typed leaf accessors (`as_int64` is Rust only), and every
Arrow crossing is a copy. Cross once - `asJs()` for plain values or
`intoArrowArray()` for a typed Arrow JS vector - then compute in JavaScript;
never call `scalar(i)` in a hot loop.

```javascript
const assert = require('node:assert/strict')
const { Serie, fields } = require('yggdryl')

const prices = Serie.fromScalars(fields.int64('price'), [125n, null, 127n])

// One crossing, then plain JavaScript.
const values = prices.asJs()
assert.equal(values.reduce((sum, value) => sum + (value ?? 0), 0), 252)

// Or one typed vector: int64 is a BigInt64Array under a validity bitmap.
const vector = prices.intoArrowArray()
assert.equal(vector.nullCount, 1)
assert.deepEqual(vector.toArray(), BigInt64Array.from([125n, 0n, 127n]))
assert.equal(vector.get(1), null)
```

## Read and write rows, children and slices

Every write is spelled over `splice`, proves its rows through the field
first and leaves the column unchanged on refusal. Nested cells are written by
path; no child is handed out mutably.

```javascript
const assert = require('node:assert/strict')
const { Field, Serie, fields } = require('yggdryl')

const prices = Serie.fromScalars(fields.int64('price'), [1n])
prices.push(2n)
prices.extend([3n, null])
prices.set(0, 10n)
prices.insert(1, 5n)
assert.equal(prices.remove(1).asJs(), 5)
assert.equal(prices.pop().asJs(), null)
prices.splice(0, 1, [7n, 8n])
assert.deepEqual(prices.asJs(), [7, 8, 2, 3])
assert.equal(prices.scalar(1).asJs(), 8)
assert.equal(prices.at(99), null)
assert.deepEqual(prices.slice(1, 2).asJs(), [8, 2])
prices.truncate(2)
assert.deepEqual([...prices].map((row) => row.asJs()), [7, 8])

const root = Field.from('row: struct<id: int64 not null, tags: serie<item: utf8 not null>> not null')
const rows = Serie.fromScalars(root, [[1n, ['a', 'b']], [2n, null]])
rows.setCell('id', 1, 20n)
assert.deepEqual(rows.child('id').asJs(), [1, 20])
assert.deepEqual(rows.getChildByPath('tags.item').asJs(), ['a', 'b'])
assert.deepEqual(rows.child('tags').offsets, [0, 2, 2])

assert.throws(() => rows.push([3n, 'not a list']))
assert.equal(rows.length, 2) // unchanged
```

## Keep chunks and batches apart

`ChunkedSerie` holds a vector's `Data` or a table's batches without
concatenating; `intoSerie()` is the one join. There is no constructor: build
one through a static.

```javascript
const assert = require('node:assert/strict')
const arrow = require('apache-arrow')
const { ChunkedSerie, Serie, fields } = require('yggdryl')

const int64 = (values) => arrow.vectorFromArray(values, new arrow.Int64())
const price = fields.int64('price', { nullable: false })
const prices = ChunkedSerie.fromArrowArray(int64([125n, 126n]).concat(int64([127n])), price)
assert.deepEqual([prices.length, prices.numChunks], [3, 2])
assert.equal(prices.scalar(2).asJs(), 127)
assert.equal(prices.slice(1, 2).numChunks, 2)
assert.equal(prices.intoArrowArray().data.length, 2)

const quotes = (ids, symbols) =>
  new arrow.Table({ id: int64(ids), symbol: arrow.vectorFromArray(symbols, new arrow.Utf8()) })
const table = new arrow.Table([
  ...quotes([1n, 2n], ['AAPL', 'MSFT']).batches,
  ...quotes([3n], ['AMD']).batches,
])
const held = ChunkedSerie.fromArrowBatch(table)
assert.equal(held.numChunks, 2)
assert.deepEqual(held.child('symbol').asJs(), ['AAPL', 'MSFT', 'AMD'])
assert.deepEqual(held.intoArrowTable().batches.map((batch) => batch.numRows), [2, 1])

assert.equal(held.cast(fields.struct('row', [fields.float64('id'), fields.utf8('symbol')], { nullable: false })).numChunks, 2)
const joined = held.intoSerie() // the one concatenation
assert.ok(joined instanceof Serie && joined.equals(held))
assert.throws(() => new ChunkedSerie())
```

## Hand a held column on as a stream

`SerieReader.fromSerie` and `fromChunked` read held data as a stream with no
plan and no copy. `reader.cast(field)` re-roots the stream under one plan and
consumes the reader.

```javascript
const assert = require('node:assert/strict')
const { ChunkedSerie, Field, Serie, SerieReader, fields } = require('yggdryl')

const root = Field.from('row: struct<id: int64 not null> not null')
const rows = Serie.fromScalars(root, [[1n], [2n]])
const held = [...SerieReader.fromSerie(rows)]
assert.equal(held.length, 1)
assert.ok(held[0].equals(rows))

// A leaf column is the one child of a `row` record.
const price = Serie.fromScalars(fields.int64('price'), [1n, 2n])
assert.deepEqual([...SerieReader.fromSerie(price)][0].child('price').asJs(), [1, 2])

const chunks = ChunkedSerie.fromSeries([price, price])
assert.deepEqual([...SerieReader.fromChunked(chunks)].map((record) => record.length), [2, 2])

const wide = SerieReader.fromSerie(rows).cast(Field.from('row: struct<id: float64 not null> not null'))
assert.deepEqual([...wide][0].child('id').asJs(), [1, 2])
```

## Hold a column as one value

`intoScalar()` makes a column one serie `Scalar`, and `asSerie()` reaches the
column again; neither reads a row. A stream is never a `Scalar`.

```javascript
const assert = require('node:assert/strict')
const { Serie, fields } = require('yggdryl')

const column = Serie.fromScalars(fields.int64('price', { nullable: false }), [125n, 126n])
const value = column.intoScalar()
assert.equal(value.kind, 'serie')
assert.ok(value.asSerie().equals(column))

// Identity is the rows alone: not the field, not the width.
assert.ok(column.equals(new Serie([125n, 126n])))
assert.ok(column.equals(Serie.fromScalars(fields.int32('size'), [125, 126])))
```

## Convert schemas and types

JavaScript has no schema projection of its own. Read an Arrow JS schema
exactly - nullability included - through the IPC doors (`BatchReader.from`,
`Serie.fromArrowBatch`, `ArrowCastPlan.compile`); `DataType.fromArrow` and
`Field.fromArrow` read an Arrow JS value's text, which states no nullability.

```javascript
const assert = require('node:assert/strict')
const arrow = require('apache-arrow')
const { BatchReader, DataType, Field } = require('yggdryl')

const schema = new arrow.Schema([new arrow.Field('id', new arrow.Int64(), false)])
const data = arrow.makeData({
  type: new arrow.Struct(schema.fields),
  length: 1,
  nullCount: 0,
  children: [arrow.makeData({ type: new arrow.Int64(), data: BigInt64Array.from([1n]) })],
})
const table = new arrow.Table([new arrow.RecordBatch(schema, data)])

const root = BatchReader.from(table).field
assert.ok(root.equals(Field.from('row: struct<id: int64 not null> not null')))

assert.equal(DataType.fromArrow(new arrow.Int32()).toString(), 'int32')
assert.equal(Field.fromArrow(schema.fields[0]).nullable, true) // the text drops `not null`
```

## Merge two streams

`left.combined(right, schema?)` chains two readers onto the root their
schemas merge into, lazily; both are consumed.

```javascript
const assert = require('node:assert/strict')
const arrow = require('apache-arrow')
const { BatchReader } = require('yggdryl')

const left = BatchReader.from(new arrow.Table({ id: arrow.vectorFromArray([1n], new arrow.Int64()) }))
const right = BatchReader.from(
  new arrow.Table({
    id: arrow.vectorFromArray([2n], new arrow.Int64()),
    venue: arrow.vectorFromArray(['XPAR'], new arrow.Utf8()),
  }),
)

const joined = left.combined(right).intoTable()
assert.equal(joined.numRows, 2)
assert.deepEqual(joined.schema.fields.map((field) => field.name), ['id', 'venue'])
assert.deepEqual([...joined.getChild('venue')], [null, 'XPAR'])
```

## Gotchas in JavaScript

- Nothing is zero copy: every vector, batch or table crosses as one IPC
  stream. `Serie.fromArrowArray(vector)` with several `Data` casts them once
  as one column; use `ChunkedSerie.fromArrowArray` to keep them apart.
- `int64` values go in as `bigint`; `asJs()` answers a `number` where the
  value is a safe integer and a `bigint` past 2^53, and `intoArrowArray()` a
  `BigInt64Array`-backed vector.
- `fromArrowReader` and `SerieReader.fromArrowReader` take only a native
  `BatchReader` and consume it; convert with `BatchReader.from(value)`.
- A `SerieReader` is read once: iterating it, `cast` and `intoArrowReader`
  each consume it.
- `new ChunkedSerie()` throws; `new Serie(rows)` builds a schema-free run,
  not a column - use `Serie.fromScalars(field, rows)` for a column.
- Arrow JS holds no vector of no `Data`, so an empty `ChunkedSerie` goes out
  as a vector of one empty `Data` and crosses back as one empty chunk.
