# yggdryl-types in JavaScript

`const { DataType, Field, Scalar, fields, intoField } = require('yggdryl')`
(CommonJS; types ship in the package). `fields.*` factories return the one
native `Field`, nullable unless `{ nullable: false }`; methods are camelCase
over the same core, and 64-bit integers and decimal coefficients are `bigint`.

## Parse a datatype once and read it back

`DataType.from(text)` (or `new DataType(text)`) parses any spelling once;
`toString()` is the canonical text and round-trips.

```javascript
const assert = require('node:assert/strict')
const { DataType } = require('yggdryl')

const amount = DataType.from('numeric(18, 4)')
assert.equal(amount.toString(), 'decimal64(18,4)')   // one canonical spelling
assert.equal(amount.id, 'decimal64')
assert.equal(amount.kind, 'decimal')
assert.ok(DataType.fromString(amount.toString()).equals(amount))

// Every dialect's spelling is the same value: compare values, not text.
assert.ok(DataType.from('bigint').equals(DataType.from('int64')))
assert.ok(DataType.from('list<int64>').equals(DataType.from('array<int64>')))
assert.equal(DataType.from('timestamp').toString(), 'datetime64(us)')

// An Arrow JS type is read through its own textual form.
assert.equal(DataType.fromArrow({ toString: () => 'Int32' }).id, 'int32')

// Refusals name the byte where parsing stopped.
assert.throws(() => DataType.from('large_utf8(64)'), /at byte/)
```

## Put the width, unit, scale and zone on the type

`DataType.time(unit)` picks the time width; a decimal is declared on the
field (`fields.decimal(name, p, s)`) or in text - there is no
`DataType.decimal`. Strings and bytes name their leaf.

```javascript
const assert = require('node:assert/strict')
const { DataType, fields } = require('yggdryl')

assert.equal(DataType.time('ms').toString(), 'time32(ms)')
assert.equal(DataType.time('ns').toString(), 'time64(ns)')
assert.equal(fields.decimal('amount', 9, 2).dtype.toString(), 'decimal32(9,2)')
assert.equal(fields.decimal('amount', 38, 4).dtype.toString(), 'decimal128(38,4)')
assert.equal(fields.datetime64('at', 'ns', 'UTC').dtype.toString(), 'datetime64(ns,"UTC")')

// The leaf is the whole string declaration: charset, shape, number.
const latin = DataType.string({ charset: 'windows-1252', max: 32 })
assert.equal(latin.toString(), 'sized_cp1252(32)')
assert.equal(latin.stringParameters.layout, 'sized_cp1252')
assert.ok(DataType.from('varchar(32)').equals(DataType.from('sized_utf8(32)')))
assert.equal(DataType.fixedAscii(4).fixedByteWidth, 4)
assert.equal(DataType.bytes({ max: 16 }).toString(), 'sized_binary(16)')
assert.equal(DataType.fixedSizeBinary(16).fixedByteWidth, 16)

// The bare word `decimal` is the fixed leaf, not a precision.
assert.equal(DataType.from('decimal').id, 'decimal')
assert.equal(fields.decimal('px').dtype.toString(), 'decimal')
```

## Build a field and a schema

A schema is a non-null struct `Field`. JavaScript has no subscripts: children
are reached with `field`, `getFieldAt`, `getFieldByPath` and `indexOf`.

```javascript
const assert = require('node:assert/strict')
const { DataType, Field, fields } = require('yggdryl')

const trade = fields.struct('trade', [
  fields.int64('id', { nullable: false }),
  new Field('price', 'decimal(18, 4)', false),
  fields.struct('venue', [fields.mic('mic')]),
], { nullable: false })

assert.equal(trade.nullable, false)
assert.equal(trade.field('id').dtype.toString(), 'int64')
assert.equal(trade.getFieldAt(1).name, 'price')
assert.equal(trade.getFieldByPath('venue.mic').dtype.id, 'mic')
assert.equal(trade.indexOf('price'), 1)
assert.deepEqual(trade.dtype.keys(), ['id', 'price', 'venue'])
assert.equal(trade.getField('missing'), null)

// The same field from text.
assert.ok(Field.from('id int64 NOT NULL').equals(trade.field('id')))
assert.ok(DataType.fromFields([fields.utf8('a')]).equals(DataType.from('struct<a:utf8>')))
```

## Declare a schema from a class

A class exposes its root through a static getter `intoStructField`;
`intoField(value, name?)` validates it, memoizes it per class, and answers the
same field for the class and its instances.

```javascript
const assert = require('node:assert/strict')
const { Field, fields, intoField } = require('yggdryl')

let builds = 0
class Trade {
  static get intoStructField() {
    builds += 1
    return fields.struct('Trade', [
      fields.int64('id', { nullable: false }),
      fields.utf8('symbol'),
      fields.decimal128('price', 18, 4),
    ], { nullable: false })
  }
}

const root = intoField(Trade)
assert.ok(root instanceof Field)
assert.equal(intoField(new Trade()), root)          // one cached field
assert.equal(builds, 1)
assert.equal(intoField(Trade, 'trades').name, 'trades')   // a renamed clone
assert.equal(root.name, 'Trade')

// A row under it is the ordered children; a plain object is named input.
assert.deepEqual(root.scalar({ symbol: 'AAPL', id: 7n, price: '1.5' }).get(0).asJs(), 7)
```

## Read a value through a column

`field.scalar(v)` / `dtype.scalar(v)` is the one value door: it narrows to
the declared width, parses text and applies nullability.

```javascript
const assert = require('node:assert/strict')
const { DataType, Field, Scalar, fields } = require('yggdryl')

const qty = new Field('qty', 'int16', false)
const value = qty.scalar(42)
assert.equal(value.kind, 'i16')                     // narrowed to the column
assert.equal(value.id, 'int16')
assert.ok(qty.scalar('42').equals(value))           // text reads under the type

assert.throws(() => qty.scalar(null), /non-nullable field received null/)
assert.throws(() => qty.scalar(70000), /expected int16/)   // never wraps

// 100 and 100.0 are one Number: an integral float needs Scalar.float.
assert.throws(() => new DataType('float64').scalar(100), /expected float64/)
assert.equal(new DataType('float64').scalar(Scalar.float(100)).kind, 'f64')

// Past 2^53 a Number is a float; pass a bigint to a 64-bit integer column.
assert.throws(() => fields.int64('id').scalar(2 ** 53 + 2))
assert.equal(fields.int64('id').scalar(2n ** 60n).asJs(), 2n ** 60n)
assert.equal(typeof fields.int64('id').scalar(7n).asJs(), 'number')
```

## Build a row

A row is the ordered sequence of the struct's children. A plain object is a
named record and a child it does not name takes its default; an array is
positional.

```javascript
const assert = require('node:assert/strict')
const { Scalar, fields } = require('yggdryl')

const trade = fields.struct('trade', [
  fields.int64('id', { nullable: false }),
  fields.utf8('symbol'),
], { nullable: false })

assert.deepEqual(trade.scalar({ symbol: 'AAPL', id: 7n }).asJs(), [7, 'AAPL'])
assert.deepEqual(trade.scalar([7n, 'AAPL']).asJs(), [7, 'AAPL'])
assert.deepEqual(trade.scalar({ id: 7n }).asJs(), [7, null])

const row = trade.scalar({ id: 7n, symbol: 'AAPL' })
assert.equal(row.length, 2)
assert.equal(row.get(1).asJs(), 'AAPL')
assert.equal(Scalar.from({ id: 1 }).kind, 'struct')   // an object is a record
assert.equal(Scalar.from(new Map([['id', 1]])).kind, 'map')
```

## Infer a value and convert it back

`Scalar.from(v)` reads what a JavaScript value is - a `Number` integer is
`i64`, a fraction `f64`, a `Date` `datetime64(ms,"UTC")` - and `asJs()` is the
way back. Equality and `stableHash()` are by value across widths.

```javascript
const assert = require('node:assert/strict')
const { DataType, Scalar } = require('yggdryl')

const seven = Scalar.from(7)
assert.equal(seven.kind, 'i64')
assert.equal(seven.family, 'integer')
assert.equal(Scalar.from(1.5).kind, 'f64')
assert.equal(Scalar.from(Buffer.from([1])).family, 'bytes')
assert.equal(Scalar.from(new Date(0)).dtype.toString(), 'datetime64(ms,"UTC")')
assert.equal(Scalar.from(2n ** 70n).kind, 'i128')

// One number at two widths is one value and one stable hash.
const narrow = DataType.from('uint8').scalar(7)
assert.ok(narrow.equals(seven))
assert.equal(narrow.stableHash(), seven.stableHash())   // a bigint
assert.equal(Scalar.from(1).compare(Scalar.from(2)), -1)

assert.equal(seven.asJs(), 7)
assert.deepEqual(Scalar.from([1, null]).asJs(), [1, null])
assert.equal(Scalar.from(42).intoField().name, 'value')   // the inferred field
```

## Decimals, durations, floats and checked arithmetic

`Scalar.decimal(coefficient: bigint, scale)`, `Scalar.duration(count, unit)`
and `Scalar.float(value, width)` are the only values the type side cannot
state. Arithmetic is named methods only, exact or thrown with an `err.code`.

```javascript
const assert = require('node:assert/strict')
const { Scalar, fields } = require('yggdryl')

const price = Scalar.decimal(1050n, 2)
assert.equal(price.kind, 'd128')
assert.equal(price.unscaled, 1050n)
assert.equal(price.scale, 2)
assert.equal(price.toString(), '"10.50"')         // asJs() answers the Scalar
assert.ok(price.equals(Scalar.decimal(105n, 1)))  // normalized equality

const amount = fields.decimal('amount', 10, 2)
assert.equal(amount.scalar('12.5').unscaled, 1250n)
assert.throws(() => amount.scalar(12.5), /unscaled decimal integer, got f64/)

assert.ok(Scalar.decimal(1n).divide(Scalar.decimal(2n)).equals(Scalar.decimal(5n, 1)))
assert.throws(() => Scalar.decimal(1n).divide(Scalar.decimal(3n)),
  (e) => e.code === 'ERR_YGGDRYL_INEXACT_ARITHMETIC')
assert.throws(() => Scalar.from(1).divide(0),
  (e) => e instanceof RangeError && e.code === 'ERR_YGGDRYL_DIVISION_BY_ZERO')
assert.equal(Scalar.from(10).divide(4).asJs(), 2)   // integer division
assert.equal(Scalar.from(40).add(2).asJs(), 42)

assert.equal(Scalar.duration(7, 'ms').kind, 'duration32')   // width from count
assert.equal(Scalar.duration(2147483648n, 'us').kind, 'duration64')
assert.equal(Scalar.float(1.5, 32).kind, 'f32')
```

## Temporal values and zones

A datetime column carries its unit and zone. A count at the column's unit, ISO
text, or (for millisecond UTC) a `Date` read into it; `count`, `unit` and
`zone` read any temporal back.

```javascript
const assert = require('node:assert/strict')
const { DataType, fields } = require('yggdryl')

const at = fields.datetime64('at', 'ns', 'UTC', { nullable: false })
const v = at.scalar('2024-01-02T00:00:00Z')
assert.equal(v.count, 1704153600000000000n)
assert.equal(v.unit, 'ns')
assert.equal(v.zone, 'UTC')
assert.ok(at.scalar(1704153600000000000n).equals(v))

// A millisecond UTC instant is a Date both ways.
const ms = fields.datetime64('at', 'ms', 'UTC').scalar(1700000000000n)
assert.ok(ms.asJs() instanceof Date)

// A Date is an instant, so a date32 column refuses it; give days or text.
assert.throws(() => new DataType('date32').scalar(new Date(0)))
assert.equal(new DataType('date32').scalar('1970-01-02').count, 1n)
assert.equal(fields.timezone('tz').scalar('Asia/Calcutta').asJs(), 'Asia/Kolkata')
```

## Strings, bytes, codes and identifiers

A bound counts stored bytes; a registered code is its own datatype with its
own validity; a UUID column reads every spelling to one value.

```javascript
const assert = require('node:assert/strict')
const { DataType, Scalar } = require('yggdryl')

const bounded = DataType.from('sized_ascii(4)')
assert.equal(bounded.scalar('USD').asJs(), 'USD')
assert.equal(bounded.scalar('USD').dtype.toString(), 'sized_ascii(4)')
assert.throws(() => bounded.scalar('EURO!'), /at most 4 bytes/)

const ccy = new DataType('ccy')
assert.equal(ccy.kind, 'code')
assert.equal(ccy.codeWidth, 3)
assert.equal(ccy.stringParameters, null)
assert.equal(ccy.scalar('USD').kind, 'ccy')
assert.ok(!ccy.scalar('USD').equals(Scalar.from('USD')))   // not a string
assert.throws(() => new DataType('isin').scalar('US0378331006'), /check digit/)

const text = '01912d68-783e-7c9a-b1f2-0123456789ab'
const uuid = new DataType('uuid')
assert.equal(uuid.scalar(text.toUpperCase()).asJs(), text)
assert.deepEqual([...DataType.from('binary(2)').scalar(Buffer.from([1, 2])).asJs()], [1, 2])
```

## Nested values: serie, map, union, dictionary

Each nested factory takes its child fields; a union value reads as
`[typeId, payload]` and a bare payload enters the one member that accepts it.

```javascript
const assert = require('node:assert/strict')
const { fields } = require('yggdryl')

const levels = fields.serie('levels', fields.float64('item'))
assert.equal(levels.dtype.id, 'serie')
assert.equal(levels.dtype.kind, 'nested')
assert.deepEqual(levels.scalar([1.5, 2.5]).asJs(), [1.5, 2.5])

const lookup = fields.mapOf('lookup', 'utf8', 'int64')
assert.deepEqual(lookup.scalar(new Map([['a', 1n]])).asJs(), new Map([['a', 1]]))
assert.deepEqual([...lookup.scalar(new Map([['a', 1n]]))].map((key) => key.asJs()), ['a'])

const payload = fields.denseUnion('p', [fields.int64('n', { nullable: false }), fields.utf8('t')])
assert.deepEqual(payload.scalar(7n).asJs(), [0, 7])
assert.deepEqual(payload.scalar('hi').asJs(), [1, 'hi'])

const codes = fields.dictionary('codes', 'int16', 'utf8')
assert.equal(codes.scalar('AAPL').dtype.toString(), 'utf8')   // decoded value
const xy = fields.fixedSizeSerie('xy', fields.float64('item'), 2)
assert.equal(xy.scalar([1.5, 2.5]).length, 2)
assert.throws(() => xy.scalar([1, 2]), /expected float64, got i64/)   // integral Numbers
```

## Metadata and protocol properties

A `Field` is a `Map`-like view of its metadata (`get`, `set`, `has`,
`delete`, `size`, iteration); typed accessors and protocol views read and
write that same map.

```javascript
const assert = require('node:assert/strict')
const { DataType, Field } = require('yggdryl')

const price = new Field('price', 'decimal(18, 4)', false, { source: 'feed' })
price.setParquetFieldId(17)
price.setComment('closing price')
price.setLocation('s3://warehouse/bars/data.arrow')
price.setProperty('postgres', 'type', 'numeric(18,4)')
price.iceberg.set('doc', 'closing price')

assert.equal(price.parquetFieldId, 17)
assert.equal(price.get('PARQUET:field_id'), '17')
assert.equal(price.comment, 'closing price')
assert.equal(price.location.scheme, 's3')
assert.equal(price.getProperty('postgres', 'type'), 'numeric(18,4)')
assert.equal(price.get('ICEBERG:doc'), 'closing price')
assert.equal(price.get('source'), 'feed')
assert.equal(price.getField('source'), null)          // a key, not a child

const row = new Field('row', DataType.from('struct<year:int32 not null,px:float64>'), false)
  .withPartitionFields(['year'])
assert.deepEqual(row.partitionFieldNames(), ['year'])
assert.equal(row.field('year').get('FIELD:partition'), 'true')
```

## Compare, diff and merge schemas

`equals` answers yes or no, `showDiffs` why; `mergeWith` is the one promotion
table (widening by default).

```javascript
const assert = require('node:assert/strict')
const { DataType, Field } = require('yggdryl')

const left = new Field('price', 'float64', false, { venue: 'XPAR' })
const right = new Field('price', 'float64', true, { venue: 'XNAS' })
assert.equal(left.equals(right), false)
assert.ok(left.equals(new Field('price', 'float64', false), false))
assert.deepEqual([...left.showDiffs(right)], [
  '≠ $.nullable: false → true',
  '≠ $.metadata["venue"]: "XPAR" → "XNAS"',
])

const merged = DataType.from('struct<id:int32 not null,venue:utf8>')
  .mergeWith('struct<id:int64 not null,px:float64>')
assert.equal(merged.getField('id').dtype.toString(), 'int64')
assert.deepEqual(merged.keys(), ['id', 'venue', 'px'])
assert.equal(merged.getField('px').nullable, true)
assert.ok(DataType.from('int32').mergeWith('int64', false).equals(DataType.from('int32')))
assert.throws(() => DataType.from('decimal(10,2)').mergeWith('float64'))
```

## Serialize a schema, move a value as bytes

JavaScript writes the schema document as JSON (`toJSON`, `toJSONBytes`,
`fromJSON`); YAML, TOML and `pretty()` are Rust and Python only. A value is
one self-describing byte stream.

```javascript
const assert = require('node:assert/strict')
const { DataType, Field, Scalar } = require('yggdryl')

const field = new Field('price', 'decimal(9, 2)', false, { venue: 'XPAR' })
assert.ok(Field.fromJSON(field.toJSON()).equals(field))
assert.ok(Field.fromJSONBytes(field.toJSONBytes()).equals(field))
assert.deepEqual(DataType.from('decimal(9,2)').toJSON(), { type: 'decimal32', precision: 9, scale: 2 })

const value = new DataType('int32').scalar(7)
const data = value.intoValueBytes()
assert.equal(data[0], 0)                                // the stream version
assert.equal(Scalar.fromValueBytes(data).kind, 'i32')   // the leaf survives
const row = Scalar.from({ symbol: 'AAPL', sizes: [100, null] })
assert.ok(Scalar.fromValueBytes(row.intoValueBytes()).equals(row))
```

## Defaults and engine compatibility

The core computes one canonical default per type and projects it to plain
JavaScript; `intoSchemeCompat` applies the layout rewrites one engine needs.

```javascript
const assert = require('node:assert/strict')
const { DataType, fields } = require('yggdryl')

const payload = fields.struct('payload', [
  fields.int32('count', { nullable: false }),
  fields.utf8('note'),
], { nullable: false })
assert.deepEqual(payload.defaultJSValue(), [0, null])
assert.equal(new DataType('utf8').defaultJSValue(), '')

assert.equal(fields.uint8('small').intoSchemeCompat('spark').dtype.id, 'int16')
assert.throws(() => DataType.from('datetime64(ns)').intoSchemeCompat('spark'), /got ns/)
```

## Gotchas in JavaScript

- Pass `bigint` for `int64`/`uint64` values and decimal coefficients;
  `Scalar.decimal(1250, 2)` with a `Number` throws. `asJs()` gives a `Number`
  when it fits and a `bigint` when it does not.
- An integral `Number` is an integer: `new DataType('float64').scalar(100)`
  is refused, and so is `[1, 2]` under a `float64` item - use
  `Scalar.float(100)` or a non-integral value.
- `asJs()` returns the `Scalar` itself where JavaScript has no spelling
  (decimals, nanosecond or zoned instants, durations): read `unscaled`,
  `scale`, `count`, `unit`, `zone`, or `toString()`.
- A plain object is a record (struct); a `Map` is a map. Union values read
  back as `[typeId, payload]`.
- No subscripts and no `DataType.decimal`: use `field('x')`,
  `getFieldAt(i)`, `getFieldByPath('a.b')`, and `fields.decimal*`.
- `field.get('x')` is metadata; `field.field('x')` / `getField('x')` is a
  child.
- `DataType.kind` is the family (`DataType.time('ms').kind === 'temporal'`);
  the leaf is `id` (`'time32'`).
- No `validateStructRoot`, `applyArrowBatch`, `pretty`, YAML/TOML schema
  writers, `uuidPacked`, `FieldScalar` or `FieldRecord`: JavaScript validates
  at every entry point, and the rest is Rust (and Python) only.
- Arrow JS crossing is copied IPC (see `yggdryl-arrow`); Arrow JS rows carry no
  extension identity, so a code column read through Arrow JS is plain text.
