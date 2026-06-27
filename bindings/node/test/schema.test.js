// Tests for the yggdryl schema + temporal types (DataType, Field, Date, Time,
// DateTime, Duration, Timezone). Build first with `npm run build`, then `node --test`.

const { test } = require('node:test')
const assert = require('node:assert')
const { DataType, Field, Date: YDate, Time, DateTime, Duration, Timezone } = require('..')

test('datatype parse and constructors', () => {
  assert.ok(DataType.fromStr('int64').equals(DataType.int(64)))
  assert.ok(DataType.int(8, false).equals(new DataType('uint8')))
  assert.ok(new DataType('string').equals(DataType.varchar()))
  assert.strictEqual(DataType.float(64).toString(), 'float64')
  assert.strictEqual(DataType.decimal(10, 2).toString(), 'decimal128[10, 2]')
  assert.strictEqual(DataType.timestamp('us', 'UTC').toString(), 'timestamp[us, UTC]')
})

test('datatype accessors and categories', () => {
  assert.strictEqual(DataType.int(32).category, 'primitive')
  assert.strictEqual(DataType.date().category, 'logical')
  assert.strictEqual(DataType.struct([]).category, 'nested')
  assert.strictEqual(DataType.any().category, 'any')
  assert.strictEqual(DataType.int(32).bitSize, 32)
  assert.strictEqual(DataType.boolean().bitSize, 1)
  assert.strictEqual(DataType.varchar().bitSize, null)
  assert.ok(DataType.varchar(undefined, true).isLarge)
  assert.strictEqual(DataType.varchar('latin1').charset, 'latin1')
  assert.strictEqual(DataType.timestamp('ns', 'Asia/Tokyo').timeUnit, 'ns')
  assert.strictEqual(DataType.timestamp('ns', 'Asia/Tokyo').timezone.name, 'Asia/Tokyo')
})

test('datatype predicates and children', () => {
  assert.ok(DataType.int(32).isSignedInteger())
  assert.ok(DataType.int(32, false).isUnsignedInteger())
  assert.ok(DataType.float(32).isNumeric())
  assert.ok(!DataType.decimal(1, 0).isNumeric())
  assert.ok(DataType.varchar().isString())
  const s = DataType.struct([
    new Field('a', DataType.int(32)),
    new Field('b', DataType.varchar()),
  ])
  assert.ok(s.isStruct())
  assert.deepStrictEqual(s.children().map((f) => f.name), ['a', 'b'])
})

test('datatype coercion and merge', () => {
  assert.ok(DataType.int(8).commonType(DataType.int(32)).equals(DataType.int(32)))
  assert.ok(DataType.int(32).commonType(DataType.float(32)).equals(DataType.float(64)))
  assert.strictEqual(DataType.int(32).commonType(DataType.varchar()), null)
  assert.ok(DataType.int(32).canCastTo(DataType.varchar()))
  assert.ok(!DataType.int(32).canCastTo(DataType.binary()))
  assert.ok(DataType.int(8).merge(DataType.int(64), 'promote').equals(DataType.int(64)))
  assert.throws(() => DataType.int(8).merge(DataType.int(64), 'strict'))
  assert.ok(DataType.int(8).merge(DataType.varchar(), 'permissive').equals(DataType.any()))
})

test('datatype serialisation roundtrips', () => {
  const dt = DataType.struct([
    new Field('id', DataType.int(64), false),
    new Field('name', DataType.varchar()),
  ])
  assert.ok(DataType.fromJSON(dt.toJSON()).equals(dt))
  assert.ok(DataType.fromMapping(dt.toMapping()).equals(dt))
  assert.ok(DataType.fromStr(dt.toString()).equals(dt))
  assert.strictEqual(Buffer.from(dt.toBytes()).toString(), dt.toString())
  assert.strictEqual(JSON.stringify(dt), JSON.stringify(dt.toJSON()))
})

test('field surface and metadata', () => {
  const f = new Field('id', DataType.int(64), false).withComment('pk')
  assert.strictEqual(f.name, 'id')
  assert.ok(!f.nullable)
  assert.ok(f.dataType.equals(DataType.int(64)))
  assert.strictEqual(f.comment, 'pk')
  assert.strictEqual(f.toString(), 'id: int64 not null')
  const m = new Field('id', DataType.int(64))
  m.setMetadata('unit', 'count')
  assert.strictEqual(m.getMetadata('unit'), 'count')
  assert.strictEqual(m.metadata.unit, 'count')
  assert.strictEqual(m.removeMetadata('unit'), 'count')
  assert.ok(Field.fromMapping(f.toMapping()).equals(f))
  assert.ok(Field.fromJSON(f.toJSON()).equals(f))
})

test('field graph', () => {
  const schema = new Field('rec', DataType.struct([
    new Field('Id', DataType.int(64), false),
    new Field('Name', DataType.varchar()),
    new Field('addr', DataType.struct([new Field('City', DataType.varchar())])),
  ]), false)
  assert.strictEqual(schema.childCount, 3)
  assert.strictEqual(schema.child('id').name, 'Id') // case-insensitive
  assert.strictEqual(schema.childExact('id'), null) // case-sensitive
  assert.strictEqual(schema.childIndex('name'), 1)
  assert.strictEqual(schema.childAt(2).name, 'addr')
  const linked = schema.withLinkedChildren()
  const addr = linked.child('addr')
  assert.strictEqual(addr.parent.name, 'rec')
  assert.strictEqual(addr.child('city').parent.name, 'addr')
  assert.strictEqual(addr.child('city').root().name, 'rec')
  assert.ok(linked.equals(schema)) // identity ignores parent
})

test('date', () => {
  const d = new YDate(2024, 2, 29)
  assert.deepStrictEqual([d.year, d.month, d.day], [2024, 2, 29])
  assert.strictEqual(d.toString(), '2024-02-29')
  assert.strictEqual(d.weekday, 4)
  assert.ok(YDate.fromStr('2024-02-29').equals(d))
  assert.strictEqual(new YDate(2024, 1, 1).compare(new YDate(2024, 2, 1)), -1)
  assert.throws(() => new YDate(2023, 2, 29))
})

test('time and duration', () => {
  const t = new Time(13, 45, 30, 250000000)
  assert.strictEqual(t.toString(), '13:45:30.250')
  assert.deepStrictEqual([t.hour, t.minute, t.second, t.nanosecond], [13, 45, 30, 250000000])
  const d = Duration.fromStr('1h30m')
  assert.strictEqual(d.asSeconds(), 5400)
  assert.strictEqual(d.toString(), '1h30m')
  assert.strictEqual(Duration.fromUnit(500, 'ms').asNanos(), 500000000n)
  assert.ok(Duration.fromSecs(-5).isNegative)
})

test('timezone dst', () => {
  assert.ok(new Timezone('UTC').isUtc)
  assert.strictEqual(new Timezone('+05:30').offsetSeconds(0), 19800)
  const ny = new Timezone('America/New_York')
  assert.strictEqual(ny.offsetSeconds(1704067200), -5 * 3600) // Jan = EST
  assert.strictEqual(ny.offsetSeconds(1719792000), -4 * 3600) // Jul = EDT
  assert.throws(() => new Timezone('Mars/Olympus'))
})

test('datetime dst conversion', () => {
  const utc = DateTime.fromStr('2024-07-01T12:00:00Z')
  assert.strictEqual(utc.epochSeconds, 1719835200)
  const ny = utc.toTimezone('America/New_York')
  assert.strictEqual(ny.hour, 8)
  assert.strictEqual(ny.toString(), '2024-07-01T08:00:00-04:00')
  assert.strictEqual(utc.toTimezone('Asia/Tokyo').hour, 21)
  assert.strictEqual(ny.epochSeconds, utc.epochSeconds)
  const local = new DateTime(2024, 7, 1, 8, 0, 0, 0, 'America/New_York')
  assert.strictEqual(local.epochSeconds, 1719835200)
  assert.strictEqual(DateTime.fromStr('2024-07-01T12:00:00').timezone, null)
  assert.strictEqual(utc.epochNanos, 1719835200000000000n)
})
