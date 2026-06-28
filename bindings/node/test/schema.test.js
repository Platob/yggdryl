// Tests for the yggdryl schema + temporal types (DataType, Field, Date, Time,
// DateTime, Duration, Timezone). Build first with `npm run build`, then `node --test`.

const { test } = require('node:test')
const assert = require('node:assert')
const { DataType, Field, Date: YDate, Time, DateTime, Duration, Timezone } = require('..')

// ---- DataType ----

test('datatype constructors, ids and categories', () => {
  assert.strictEqual(DataType.int32().typeId, 4)
  assert.strictEqual(DataType.int32().name, 'int32')
  assert.strictEqual(DataType.int32().category, 'primitive')
  assert.ok(DataType.int32().isPrimitive())
  assert.strictEqual(DataType.boolean().name, 'bool')
  assert.strictEqual(DataType.uint64().typeId, 9)
  assert.strictEqual(DataType.decimal(10, 2).category, 'logical')
  assert.ok(DataType.decimal(10, 2).isLogical())
  assert.deepStrictEqual(DataType.decimal(10, 2).decimalParts, [10, 2])
  assert.strictEqual(DataType.utf8().decimalParts, null)
  assert.strictEqual(DataType.struct([]).category, 'nested')
  assert.ok(DataType.struct([]).isNested())
  // the decimal scale defaults to 0.
  assert.ok(DataType.decimal(10).equals(DataType.decimal(10, 0)))
})

test('datatype temporal + nested children', () => {
  const ts = DataType.timestamp('us', 'UTC')
  assert.strictEqual(ts.name, 'timestamp')
  assert.strictEqual(ts.category, 'logical')
  assert.strictEqual(DataType.interval('month_day_nano').name, 'interval')
  assert.throws(() => DataType.interval('nope'))
  // nested types expose their child fields; scalars/logicals have none.
  const s = DataType.struct([
    new Field('a', DataType.int32()),
    new Field('b', DataType.utf8()),
  ])
  assert.ok(s.isNested())
  assert.deepStrictEqual(s.fields().map((f) => f.name), ['a', 'b'])
  assert.deepStrictEqual(DataType.int32().fields(), [])
  assert.strictEqual(DataType.list(new Field('item', DataType.int32())).fields()[0].name, 'item')
})

test('datatype equals, hash and toString', () => {
  assert.ok(DataType.int64().equals(DataType.int64()))
  assert.ok(!DataType.int64().equals(DataType.int32()))
  assert.strictEqual(DataType.int64().hashCode(), DataType.int64().hashCode())
  assert.strictEqual(DataType.int32().toString(), 'int32')
})

// ---- Field ----

test('field surface and in-place mutation', () => {
  const f = new Field('id', DataType.int64())
  assert.strictEqual(f.name, 'id')
  assert.ok(f.dtype.equals(DataType.int64()))
  // name / dtype are mutable in place.
  f.name = 'ident'
  f.dtype = DataType.int32()
  assert.strictEqual(f.name, 'ident')
  assert.ok(f.dtype.equals(DataType.int32()))
  // raw byte metadata (Buffer keyed).
  f.setMetadata(Buffer.from('unit'), Buffer.from('count'))
  assert.strictEqual(f.getMetadata(Buffer.from('unit')).toString(), 'count')
  assert.strictEqual(f.removeMetadata(Buffer.from('unit')).toString(), 'count')
  assert.strictEqual(f.getMetadata(Buffer.from('unit')), null)
})

test('field reserved metadata accessors', () => {
  const f = new Field('x', DataType.int32())
  assert.strictEqual(f.comment, null)
  assert.strictEqual(f.indexName, null)
  assert.strictEqual(f.indexLevel, null)
  // setters mutate the metadata map in place.
  f.comment = 'a note'
  f.indexName = 'idx'
  f.indexLevel = 7
  assert.strictEqual(f.comment, 'a note')
  assert.strictEqual(f.indexName, 'idx')
  assert.strictEqual(f.indexLevel, 7)
  // stored under the reserved byte keys.
  assert.strictEqual(f.getMetadata(Buffer.from('comment')).toString(), 'a note')
  assert.strictEqual(f.getMetadata(Buffer.from('index_level')).toString(), '7')
  // clearing a key with null removes it, leaving the others untouched.
  f.comment = null
  f.indexLevel = null
  assert.strictEqual(f.comment, null)
  assert.strictEqual(f.indexLevel, null)
  assert.strictEqual(f.indexName, 'idx')
})

test('field equals + hash', () => {
  const a = new Field('id', DataType.int64())
  const b = new Field('id', DataType.int64())
  assert.ok(a.equals(b))
  assert.strictEqual(a.hashCode(), b.hashCode())
  b.comment = 'x'
  assert.ok(!a.equals(b))
})

// ---- temporal ----

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
  // asMillis / asMicros / fromMicros parity.
  assert.strictEqual(Duration.fromSecs(2).asMillis(), 2000n)
  assert.strictEqual(Duration.fromMicros(1500).asMicros(), 1500n)
  assert.strictEqual(Duration.fromMicros(1500).asNanos(), 1500000n)
  // Time.nanosOfDay (BigInt).
  assert.strictEqual(new Time(0, 0, 1).nanosOfDay, 1000000000n)
})

test('timezone dst', () => {
  assert.ok(new Timezone('UTC').isUtc)
  assert.strictEqual(new Timezone('+05:30').offsetSeconds(0), 19800)
  const ny = new Timezone('America/New_York')
  assert.strictEqual(ny.offsetSeconds(1704067200), -5 * 3600) // Jan = EST
  assert.strictEqual(ny.offsetSeconds(1719792000), -4 * 3600) // Jul = EDT
  assert.throws(() => new Timezone('Mars/Olympus'))
})

test('temporal math, empty default, from_datetime', () => {
  // Empty string decodes to the zero default.
  assert.strictEqual(YDate.fromStr('').toString(), '1970-01-01')
  assert.strictEqual(DateTime.fromStr('').epochSeconds, 0)
  assert.strictEqual(Duration.fromStr('').asSeconds(), 0)
  // Duration scale.
  assert.strictEqual(Duration.fromSecs(5).mul(3).asSeconds(), 15)
  assert.strictEqual(Duration.fromSecs(20).div(5).asSeconds(), 4)
  // DateTime arithmetic + diff + truncate.
  const dt = DateTime.fromStr('2024-07-01T12:00:00Z')
  const later = dt.add(Duration.fromStr('1h30m'))
  assert.strictEqual(later.toString(), '2024-07-01T13:30:00Z')
  assert.strictEqual(later.durationSince(dt).asSeconds(), 5400)
  assert.strictEqual(
    dt.add(Duration.fromStr('25m')).truncate(Duration.fromStr('1h')).toString(),
    '2024-07-01T12:00:00Z',
  )
  // Time wraps around midnight; Date adds whole days.
  assert.strictEqual(new Time(23, 30, 0).add(Duration.fromStr('1h')).toString(), '00:30:00')
  assert.strictEqual(new YDate(2024, 7, 1).add(Duration.fromStr('2d')).toString(), '2024-07-03')
  // Temporal.fromDatetime redirect.
  assert.ok(YDate.fromDatetime(dt).equals(new YDate(2024, 7, 1)))
  assert.ok(Time.fromDatetime(dt).equals(new Time(12, 0, 0)))
})

test('temporal conversions and parse', () => {
  const d = new YDate(2024, 7, 1)
  assert.strictEqual(d.toDatetime().hour, 0)
  const ny = d.withTimezone('America/New_York')
  assert.strictEqual(ny.timezone.name, 'America/New_York')
  assert.strictEqual(ny.at(new Time(8, 0, 0)).epochSeconds, 1719835200)
  assert.strictEqual(new Time(13, 30, 0).toDatetime().hour, 13)
  // fromStr is the single, strict parser (throws on malformed input; no `parse`).
  assert.strictEqual(DateTime.fromStr('2024-07-01').toString(), '2024-07-01T00:00:00')
  assert.strictEqual(DateTime.fromStr('1719835200').epochSeconds, 1719835200)
  assert.throws(() => YDate.fromStr('not-a-date'))
  // Duration ISO-8601.
  assert.strictEqual(Duration.fromStr('PT15M').asSeconds(), 900)
  assert.strictEqual(Duration.fromStr('P1D').asSeconds(), 86400)
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
