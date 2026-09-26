# yggdryl-expressions in JavaScript

`Term`, `Filter`, `Selector`, `Plan`, `Expression`, `FieldPath`, `Records`
and `BatchReader` come from `require('yggdryl')`; Arrow crosses as
`apache-arrow` tables and batches through a copied IPC stream.

## Parse a predicate, bind it once, answer rows

`bind` resolves names and converts each literal into its column's type once.
A row is a `Scalar` sequence in schema order; an exact decimal is
`Scalar.decimal`, never a JavaScript number.

```javascript
const assert = require('node:assert/strict')
const { Field, Filter, Scalar } = require('yggdryl')

const schema = new Field('trades', 'struct<ccy:utf8,price:decimal(9,2),size:bigint>', false)
const filter = new Filter("ccy = 'EUR' and price > 100")
assert.deepEqual(filter.columns, ['ccy', 'price'])

const bound = filter.bind(schema)
assert.equal(bound.term.toString(), "ccy = 'EUR' and price > decimal32(9,2) '100.00'")

const price = Scalar.decimal(15000n, 2)
assert.equal(bound.matches(Scalar.from(['EUR', price, 5])), true)
assert.equal(bound.matches(Scalar.from(['USD', price, 5])), false)
// A null makes the answer unknown, and unknown does not keep the row.
assert.ok(bound.eval(Scalar.from(['EUR', null, 5])).equals(Scalar.from(null)))
assert.equal(bound.matches(Scalar.from(['EUR', null, 5])), false)
```

## Build a term without text, with a parameter

Comparison builders take a `Term` or term text; arithmetic builders (`add`,
`subtract`, `multiply`, `divide`, `remainder`) read plain JavaScript values as
literals. A `:name` parameter is supplied once at `bind`.

```javascript
const assert = require('node:assert/strict')
const { Field, Scalar, Term } = require('yggdryl')

const schema = new Field('trades', 'struct<ccy:utf8,price:decimal(9,2),size:bigint>', false)

const eur = Term.literal(Scalar.from('EUR'))
const composed = Term.column('price').gt('100').and(Term.column('ccy').eq(eur))
assert.ok(composed.equals("price > 100 and ccy = 'EUR'"))
assert.equal(Term.column('size').add(1).multiply(2).toString(), '(size + 1) * 2')
assert.equal(Term.column('size').between('1', '10').toString(), 'size between 1 and 10')

const late = new Term('size >= :floor')
assert.deepEqual(late.parameters, ['floor'])
const bound = late.bind(schema, { floor: 10 })
assert.equal(bound.term.toString(), 'size >= 10')
assert.equal(bound.matches(Scalar.from([null, null, 11])), true)
// A missing parameter is refused; an extra one is silently ignored.
assert.throws(() => late.bind(schema, {}), /:floor/)
assert.equal(late.bind(schema, { floor: 10, typo: 1 }).term.toString(), 'size >= 10')

assert.equal(new Term('a = 1 or a = 2').simplify().toString(), 'a in (1, 2)')
```

## Filter and project an Arrow batch

`applyArrowBatch` takes and answers an Apache Arrow JS `RecordBatch`;
`Selector` computes, renames, casts and excludes.

```javascript
const assert = require('node:assert/strict')
const arrow = require('apache-arrow')
const { Field, Filter, Selector } = require('yggdryl')

const root = new Field('rows', 'struct<ccy:utf8,size:int64,secret:utf8>', false)
const batch = new arrow.Table({
  ccy: arrow.vectorFromArray(['EUR', 'USD', 'EUR'], new arrow.Utf8()),
  size: arrow.vectorFromArray([5n, 5n, 0n], new arrow.Int64()),
  secret: arrow.vectorFromArray(['a', 'b', 'c'], new arrow.Utf8()),
}).batches[0]

const kept = new Filter("ccy = 'EUR' and size > 1").applyArrowBatch(batch)
assert.deepEqual([...kept.getChild('ccy')], ['EUR'])

const selector = new Selector('* exclude (secret), size * 2 as doubled int32')
const projected = selector.applyArrowBatch(batch)
assert.deepEqual(projected.schema.fields.map((field) => field.name), ['ccy', 'size', 'doubled'])
assert.deepEqual([...projected.getChild('doubled')], [10, 10, 0])
// The published schema is known without data.
assert.equal(String(selector.applyField(root).dtype.getFieldAt(2).dtype), 'int32')
assert.equal(Selector.all().bind(root).isIdentity, true)
```

## Shape a stream with a plan

`applyArrowReader` binds once and wraps a native `BatchReader` lazily;
`applyArrow` keeps the input kind (batch, table or reader).

```javascript
const assert = require('node:assert/strict')
const arrow = require('apache-arrow')
const { BatchReader, Expression, Plan } = require('yggdryl')

const table = new arrow.Table({
  ccy: arrow.vectorFromArray(['A', 'B', 'C', 'D'], new arrow.Utf8()),
  size: arrow.vectorFromArray([1n, 4n, 3n, null], new arrow.Int64()),
})

const plan = new Plan('select ccy, size as quantity where size >= 2 order by size desc limit 1')
const reader = plan.applyArrowReader(BatchReader.from(table))
assert.ok(reader instanceof BatchReader)
assert.deepEqual([...reader.intoTable().getChild('quantity')], [4n])
assert.ok(arrow.isArrowTable(plan.applyArrow(table)))

const steps = new Expression('where size is not null; select upper(ccy) as ccy')
assert.equal(steps.kind, 'sequence')
assert.deepEqual([...steps.applyArrowBatch(table.batches[0]).getChild('ccy')], ['A', 'B', 'C'])
```

## Evaluate native rows

`applyRecords(rows, schema)` binds once and yields canonical row `Scalar`s;
plain objects are named input, canonicalized against the schema.

```javascript
const assert = require('node:assert/strict')
const { Expression, Field } = require('yggdryl')

const root = new Field('rows', 'struct<ccy:utf8,size:int64>', false)
const steps = new Expression('where size > 1; select upper(ccy) as ccy')

const records = steps.applyRecords([{ ccy: 'a', size: 1n }, { ccy: 'b', size: 2n }], root)
assert.equal(records.field.dtype.getFieldAt(0).name, 'ccy')
assert.deepEqual([...records].map((row) => row.asJs()), [['B']])
```

## Use the bound tiers directly: filter a batch or a stream

A `Bound` answers one row (`eval`, `matches`) and filters a batch, table or
reader (`filterArrowBatch`, `filterArrowTable`, `filterArrowReader`,
`filterArrow`) from the same compiled tree.

```javascript
const assert = require('node:assert/strict')
const arrow = require('apache-arrow')
const { BatchReader, Field, Filter } = require('yggdryl')

const schema = new Field('trades', 'struct<ccy:utf8,size:int64>', false)
const bound = new Filter("ccy = 'EUR' and size > 10").bind(schema)
assert.deepEqual(bound.columnIndices, [0, 1])

const table = new arrow.Table({
  ccy: arrow.vectorFromArray(['EUR', 'USD'], new arrow.Utf8()),
  size: arrow.vectorFromArray([25n, 25n], new arrow.Int64()),
})
assert.equal(bound.filterArrowBatch(table.batches[0]).numRows, 1)
assert.equal(bound.filterArrow(table).numRows, 1)
assert.equal(bound.filterArrowReader(BatchReader.from(table)).intoTable().numRows, 1)
```

## Skip a file from its statistics

Not bound in JavaScript: `Bounds`, `statistics_prune` and
`statistics_certainty` are Rust and Python only. Push the filter into the read
instead (below) and the media prunes with the same predicate.

## Split a predicate into partition and row halves

`partitionSplit()` answers `{ answerable, remaining }`: the conjuncts a
partition layout settles, and the residual over rows.

```javascript
const assert = require('node:assert/strict')
const { DataType, Field, Filter } = require('yggdryl')

const year = new Field('year', 'int32', false, { 'FIELD:partition': 'true' })
const schema = new Field('trades', DataType.fromFields([year, new Field('price', 'decimal(9,2)', false)]), false)

const { answerable, remaining } = new Filter('year = 2024 and price > 100').bind(schema).partitionSplit()
assert.equal(answerable.toString(), "year = int32 '2024'")
assert.equal(remaining.toString(), "price > decimal32(9,2) '100.00'")
```

## Push the filter and projection into a read

A record read takes `{ filter, select }` as option properties, or
`RecordOptions` built with `withFilter` / `withSelect`: the media prunes, and
only matching rows are copied across.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { IOBase } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
try {
  const handle = new IOBase(path.join(root, 'trades.parquet'))
  handle.overwriteRecords([{ ccy: 'EUR', size: 1n }, { ccy: 'USD', size: 5n }, { ccy: 'EUR', size: 9n }])

  const table = handle.readArrowReader({ filter: 'size > 2', select: 'ccy, size * 2 as doubled' }).intoTable()
  assert.deepEqual(table.schema.fields.map((field) => field.name), ['ccy', 'doubled'])
  assert.deepEqual([...table.getChild('doubled')], [10n, 18n])

  const options = handle.recordOptions().withFilter("ccy = 'EUR'").withSelect('size')
  assert.deepEqual([...handle.readArrowReader(options).intoTable().getChild('size')], [1n, 9n])
} finally {
  fs.rmSync(root, { recursive: true, force: true })
}
```

## Run a plan against storage

`execute()` reads the `from` target through its holder with the read sections
pushed down and answers a `BatchReader`; a write verb sends the shaped stream
to its target.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { pathToFileURL } = require('node:url')
const arrow = require('apache-arrow')
const { Plan } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
try {
  const url = pathToFileURL(path.join(root, 'trades.arrow')).href
  new Plan(`create '${url}' (id int64 not null, name utf8)`).execute().intoTable()
  const rows = new arrow.Table({
    id: arrow.vectorFromArray([1n, 2n, 3n], new arrow.Int64()),
    name: arrow.vectorFromArray(['a', 'b', 'c'], new arrow.Utf8()),
  }).batches[0]
  new Plan(`insert into '${url}'`).applyArrowBatch(rows)

  const read = new Plan(`select name from '${url}' where id > 1 order by id desc`)
  assert.deepEqual([...read.execute().intoTable().getChild('name')], ['c', 'b'])
} finally {
  fs.rmSync(root, { recursive: true, force: true })
}

assert.equal(new Plan('merge into t on (id) select id from s').toString(), 'upsert into t by (id) select id from s')
const built = new Plan().withSelect('id, name').withSource('raw').withFilter('id > 1').withLimit(10)
assert.equal(built.toString(), 'select id, name from raw where id > 1 limit 10')
```

## Address a nested value by path

`FieldPath` parses and renders the path grammar - child, position, key,
slice `[1:3]` and predicate segment `[ccy = 'EUR']` - and inside a term the
same steps are accessors (`child`, `at`, `slice`, `key`).

```javascript
const assert = require('node:assert/strict')
const { Field, FieldPath, Scalar, Term } = require('yggdryl')

const path = new FieldPath('line[-1].price as last_price')
assert.equal(path.length, 3)
assert.equal(path.columnName, 'last_price')
assert.equal(path.parent().toString(), 'line[-1]')
assert.equal(new FieldPath('line').join(0).toString(), 'line[0]')
assert.equal(new FieldPath('"a.b"').length, 1)
assert.equal(new FieldPath('a.b').length, 2)
assert.equal(new FieldPath('line[0:2]').length, 2)
assert.equal(new FieldPath("line[ccy = 'EUR'][0].price").length, 4)

const root = new Field('orders', 'struct<line:serie<struct<ccy:utf8,price:int64>>>', false)
const row = Scalar.from([[['EUR', 10n], ['USD', 12n], ['EUR', 14n]]])
assert.ok(new Term('line[-1].price').bind(root).eval(row).equals(Scalar.from(14n)))
assert.ok(new Term("line[ccy = 'EUR'][0].price").bind(root).eval(row).equals(Scalar.from(10n)))
assert.equal(Term.column('line').at(-1).child('price').toString(), 'line[-1].price')
```

## Call a user-defined function

JavaScript registers no function: the `namespace.name(...)` spelling parses
and prints, and a bind refuses it by name. Register it in Rust or Python.

```javascript
const assert = require('node:assert/strict')
const { Field, Term } = require('yggdryl')

const term = new Term('Skills.Triple(size)')
assert.equal(term.toString(), 'skills.triple(size)')
assert.equal(Term.call('skills.triple', [Term.column('size')]).toString(), 'skills.triple(size)')
assert.throws(() => term.bind(Field.from('rows: struct<size: int64> not null')), /skills\.triple/)
```

## Keep a derivation on the schema

`intoField` writes a selector as the declaration it is, each computed column
carrying `TRANSFORM:` metadata; `Selector.fromField` reads it back.
Recomputing the derivations on a batch (`Field.apply_arrow_batch` in Rust and
Python) is not bound in JavaScript.

```javascript
const assert = require('node:assert/strict')
const { Field, Selector } = require('yggdryl')

const root = new Field('rows', 'struct<ccy:utf8,size:int64>', false)
const stored = new Selector('ccy, size * 2 as doubled int32').intoField(root)
assert.equal(stored.dtype.getFieldAt(1).transform.get('expression'), 'size * 2')
assert.ok(Selector.fromField(stored).equals('ccy utf8 null, size * 2 as doubled int32 null'))
```

## Read the plan, the text and the document

Every layer prints canonical text that re-parses to the same tree, has a JSON
document, a `bigint` `stableHash()`, and draws its bound plan with `explain`.

```javascript
const assert = require('node:assert/strict')
const { Expression, Field, Plan, Selector, Term, expressionVocabularies } = require('yggdryl')

const schema = new Field('trades', 'struct<ccy:utf8,price:decimal(9,2)>', false)
const drawn = new Term('price > 100').bind(schema).explain()
assert.ok(drawn.startsWith('>') && drawn.includes('column price'))

const plan = new Plan('select ccy from t where price > 0 limit 10 offset 5')
assert.ok(new Plan(plan.toString()).equals(plan))
assert.deepEqual(plan.readColumns, ['price', 'ccy'])
assert.equal(new Plan('select * exclude (price)').readColumns, null)
assert.ok(Selector.fromJson(Selector.all().intoJson()).equals(Selector.all()))
assert.equal(typeof plan.stableHash(), 'bigint')

assert.ok(expressionVocabularies().functions.includes('coalesce'))
assert.throws(() => new Expression('price > 1'), /expected `select`, `where`/)
```

## Gotchas in JavaScript

- A string handed to a comparison builder is term **text**:
  `Term.column('ccy').eq('EUR')` compares two columns. Pass
  `Term.literal(Scalar.from('EUR'))` or `"'EUR'"`; `gt(100)` with a number
  throws - write `gt('100')`.
- `Term.literal` takes a `Scalar`, not a plain value: `Term.literal(Scalar.from(1n))`.
- A row for `matches`/`eval` is `Scalar.from([...])` in schema order; a
  plain object is refused there (it is accepted by `applyRecords`).
- Arrow JS interop is copied IPC: a kept-everything batch comes back equal,
  never the same object. Keep large streams native (`BatchReader`,
  `readArrowReader`, `applyArrowReader`) and convert once at the end.
- `stableHash()` is a `bigint`; `int64` cells read back as `bigint`.
- No `Bounds`/statistics pruning and no UDF registration in JavaScript, and
  no `Field` recompute of stored `TRANSFORM:` derivations.
- `Plan` has no `applyField`: its output schema is `plan.fieldFrom(root)`.
- `bind(root, params)` ignores a key no `:name` reads; compare against
  `parameters` when a typo must fail.
- `price > 9.5` on a `decimal` column is a `float64` literal that compares as
  text; write `price > decimal(9,2) '9.50'` or bind `Scalar.decimal(950n, 2)`.
