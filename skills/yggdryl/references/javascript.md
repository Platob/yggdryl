# yggdryl in JavaScript

`npm install yggdryl` (Node 18+). The package is CommonJS with bundled
TypeScript declarations; `apache-arrow` is its dependency, so
`require('apache-arrow')` resolves beside it. The native addon carries every
part of the core - Parquet, Iceberg, the object stores.

## Numbers, bigints, bytes

Width is the type's business. A 64-bit `Scalar`'s `asJs()` is a `number`
while it is a safe integer and a `bigint` beyond; an `int64` cell read from
records or batches is an Arrow JS value and always a `bigint`. Pass `bigint`
for 64-bit values you build, and use `Buffer`/`Uint8Array` for bytes. Plain objects
become named records, a `Map` stays a mapping.

```javascript
const assert = require('node:assert/strict')
const { DataType, Scalar } = require('yggdryl')

assert.equal(new DataType('int64').scalar(5n).asJs(), 5)
assert.equal(new DataType('int64').scalar(2n ** 62n).asJs(), 2n ** 62n)
assert.equal(Scalar.from(7).kind, 'i64')
// A record's keys are sorted: it is named input for a struct field.
assert.deepEqual(Scalar.from({ b: 1, a: 'x' }).asJs(), { a: 'x', b: 1 })
```

A record write's integer column may mix `number` and `bigint` across rows:
an integral `number` is read as the `bigint` beside it, one column staying
one Arrow type. Inside the write's first batch (65,536 rows by default,
fewer under `batchRowSize`), a non-integer `number` beside `bigint` values
in the same column throws `TypeError`, naming the column and the value,
before either reaches native code - see Errors below for what a later batch
throws instead.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { IOBase } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
try {
  const handle = new IOBase(path.join(root, 'ids.arrows'))
  // number and bigint in one column, one write: both land as bigint.
  handle.overwriteRecords([{ id: 1 }, { id: 2n }])
  const rows = [...handle.readRecords()]
  assert.deepEqual(rows.map((row) => row.id), [1n, 2n])

  assert.throws(
    () => handle.overwriteRecords([{ id: 1.5 }, { id: 2n }]),
    (error) => error instanceof TypeError && /column "id"/.test(error.message),
  )
} finally {
  fs.rmSync(root, { recursive: true, force: true })
}
```

## Options: `undefined` skips, `null` clears

An argument left `undefined` keeps its default; `null` is a value and
clears. Every record read and write takes `options` and, beside it, a plain
object of `RecordOptions` properties applied to a copy.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { IOBase } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
try {
  const handle = new IOBase(path.join(root, 'prices.arrows'))
  handle.overwriteRecords([{ id: 1n, px: 1.5 }, { id: 2n, px: 2.5 }])

  let columns = 0
  for (const batch of handle.readArrowReader({ select: 'id' })) {
    columns = batch.numCols
  }
  assert.equal(columns, 1)

  // `null` clears `select` back to `*` - both at the call site and on the
  // `RecordOptions` instance itself.
  let allColumns = 0
  for (const batch of handle.readArrowReader({ select: null })) {
    allColumns = batch.numCols
  }
  assert.equal(allColumns, 2)
} finally {
  fs.rmSync(root, { recursive: true, force: true })
}
```

## Errors

A native refusal throws an `Error` whose message is the core's, naming
expected, actual and location; checked arithmetic throws `TypeError` or
`RangeError` with an `ERR_YGGDRYL_*` `code`. A record write's own column
checks (mixed `number`/`bigint`, mixed plain objects and field-class
instances) throw a plain `TypeError` with no `code`, naming the column and
the value, from the binding rather than the core - but only inside the
write's first batch (65,536 rows by default, fewer under `batchRowSize`).
The same check failing in a later batch is instead wrapped by the native
decoder into a plain `Error` (not a `TypeError`) reading `Arrow schema
error: External error: TypeError: ...`, with `code: 'GenericFailure'`.

```javascript
const assert = require('node:assert/strict')
const { DataType, Scalar } = require('yggdryl')

assert.throws(() => new DataType('int8').scalar(1000), /int8/)
assert.throws(
  () => Scalar.from(1).add('x'),
  (error) => error instanceof TypeError && error.code === 'ERR_YGGDRYL_INVALID_ARITHMETIC',
)
```

## End to end: schema, value, records, document

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { DataType, Field, IOBase, Scalar, json } = require('yggdryl')

// The schema: a non-null struct field whose children are the columns.
const schema = new Field(
  'trade',
  DataType.fromFields([
    new Field('id', 'int64', false),
    new Field('symbol', 'utf8'),
    new Field('price', 'decimal(18,4)', false),
  ]),
  false,
)

// A value enters through its type and lands at the column's scale.
const price = schema.dtype.getFieldAt(2).dtype.scalar('12.5')
assert.equal(price.kind, 'd64')
assert.ok(price.equals(Scalar.decimal(125000n, 4)))

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
try {
  // The suffix picks the encoding: `.arrows` is an Arrow IPC stream.
  const handle = new IOBase(path.join(root, 'trades.arrows'))
  handle.overwriteRecords([{ id: 1n, symbol: 'AAPL', price: '224.62' }], { field: schema })

  const rows = [...handle.readRecords()]
  assert.equal(rows.length, 1)
  assert.equal(rows[0].symbol, 'AAPL')
  // An int64 cell is an Arrow JS value: always a bigint.
  assert.equal(rows[0].id, 1n)
} finally {
  fs.rmSync(root, { recursive: true, force: true })
}

// JSON, YAML, TOML and XML are byte-first: a Buffer out, bytes or text in.
assert.deepEqual(json.loads(json.dumps({ symbol: 'AAPL' })), { symbol: 'AAPL' })
```

## Logging

The addon makes the core's logging tree the process's logger when it loads,
and `logging` is Python's `logging` in camelCase over it. The core reports one
record per operation, never per row, under dotted names that follow its module
path (`yggdryl.iceberg.table`); a crate the build depends on reaches the tree
only at `warning` and above. A record no handler takes is written to standard
error from `WARNING` up as the terminal line: the time, the level's glyph and
name, `[thread]` (`main`, or `worker-N` in a worker), the logger, the call
site (`function:line`, else the Rust module's last segment or the file's
stem and the line), the message -
`2026-10-03 14:05:09,123 ! WARNING  [main] trades.feed open:42 › crossed`. A
handler on `logging.getLogger('yggdryl')` takes the records instead.
`logging.basicConfig()` gives the root a `StreamHandler` on standard error
writing that line, and a handler with no formatter writes it too; colour is
the core's rule - `NO_COLOR` off, else `FORCE_COLOR` or `CLICOLOR_FORCE` on,
else `TERM=dumb` off, else on for a terminal - and a
`StreamHandler`'s `colored` property states it.

| Door | Spelling |
| --- | --- |
| levels | `logging.NOTSET`, `TRACE`, `DEBUG`, `INFO`, `WARNING`, `ERROR`, `CRITICAL` (0, 5, 10, 20, 30, 40, 50); wherever a level is taken, a number or a name in any case |
| a logger | `logging.getLogger(name?)` (the root when absent): `name`, `parent`, `getChild`, `level`, `setLevel`, `getEffectiveLevel`, `isEnabledFor`, `propagate`, `disabled` and `deduplicating` properties, `addHandler`, `removeHandler`, `hasHandlers`, `isDeduplicating`, `log(level, message)`, `debug`/`info`/`warning`/`error`/`critical(message)`, `equals` |
| a handler | `new logging.StreamHandler('stderr' \| 'stdout')` (`colored` read and set), `new logging.FileHandler(location, { mode, capacity, flushLevel, level })`, `new logging.NullHandler()`; `setLevel`, `setFormatter(new logging.Formatter(format?, { datefmt, timezone }))` or `logging.Formatter.terminal()`, `flush`, `close` |
| the whole tree | `logging.basicConfig({ level, format, datefmt, handlers, force })` (the terminal line unless `format` is given), `logging.disable(level = CRITICAL)`, `logging.shutdown()` |

`FileHandler` writes through any location (`string`, `Url`, `IOBase`): one
append per publish, each record at once by default; `mode` is `'append'` or
`'overwrite'`. A `capacity` holds records back until that many bytes are held
or a record at `flushLevel` (`ERROR` by default) arrives - state one over a
remote store, where an append is a whole `PUT`. The addon runs
`logging.shutdown()` when the main thread's process exits, publishing what a
handler holds; a worker ending closes nothing, the tree being the process's.
A JavaScript callback cannot be a handler (a record can be logged on any Rust
thread, and a function runs on its isolate's thread only), and a logger's
handler list is not offered: `hasHandlers` answers.

A logger whose `deduplicating` is `true` (`false` passes every record, `null`
takes its ancestors' choice, off at the root unless set) says a record the
first time and again at its 10th, 100th, 1000th occurrence as
`message (seen N times)`; the records JavaScript logs are counted too.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { logging } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-logging-'))
try {
  const location = path.join(root, 'logs', 'feed.log')
  const handler = new logging.FileHandler(location, { capacity: 1 << 16 })
  handler.setFormatter(new logging.Formatter('%(levelname)s %(name)s %(message)s'))

  // A handler on a logger takes its children's records too.
  const feed = logging.getLogger('skills.yggdryl.logging')
  feed.setLevel('INFO')
  feed.addHandler(handler)
  assert.equal(feed.hasHandlers(), true)

  feed.info('opened')
  feed.debug('under the level')
  assert.equal(fs.existsSync(location), false) // held under the capacity
  feed.error('rejected') // a record at `flushLevel` publishes what is held
  assert.equal(
    fs.readFileSync(location, 'utf8'),
    'INFO skills.yggdryl.logging opened\nERROR skills.yggdryl.logging rejected\n',
  )

  feed.removeHandler(handler)
  handler.close()

  // A handler stating no formatter writes the terminal line, plain.
  const plainLocation = path.join(root, 'logs', 'plain.log')
  const plain = new logging.FileHandler(plainLocation)
  const book = logging.getLogger('skills.yggdryl.plain')
  book.setLevel('WARNING')
  book.propagate = false
  book.addHandler(plain)
  book.warning('crossed')
  assert.match(
    fs.readFileSync(plainLocation, 'utf8'),
    /^\d{4}-\d\d-\d\d \d\d:\d\d:\d\d,\d{3} ! WARNING  \[main\] skills\.yggdryl\.plain \S+ › crossed\n$/,
  )
  book.removeHandler(plain)
  plain.close()
} finally {
  fs.rmSync(root, { recursive: true, force: true })
}
```

## Gotchas in JavaScript

- Arrow JS interop is copied IPC with bounded cursors, never zero copy: hand whole tables or batch readers across, not one row at a time.
- `readArrowReader()` yields Arrow JS `RecordBatch`es one at a time; `readRecords()` yields plain objects whose values are Arrow JS values (a decimal is Arrow's `DecimalBigNum`, not a `number`).
- JavaScript has no decimal, so `Scalar.decimal(coefficient, scale)` and a decimal-typed `DataType.scalar('12.5')` are the ways in; `asJs()` of a decimal answers the `Scalar` itself.
- `===` compares references; use `equals`, `compare` and `stableHash()` (a `bigint`) for value semantics.
- Names are camelCased (`read_arrow_reader` is `readArrowReader`, `merge_by` is `mergeBy` / `withMergeBy`), with these exceptions: Python `Scalar.from_` is `Scalar.from`, `as_py` is `asJs`, Python `Scalar.from_struct({...})` is `Scalar.from({...})`, and `IOBase.read_arrow`/`write_arrow` are not bound (use `SerieReader.fromArrowReader(handle.readArrowReader())` and `writeArrowReader`).
