'use strict'

// Tests for the `yggdryl.temporal` value types (dates, times, timestamps, durations) and `Tz`,
// mirroring the Rust `io::fixed::temporal` suite and Python `test_temporal.py`: calendar math,
// DST-aware zone conversions, unit/timezone strings, ISO parsing, value identity, and codecs.

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { Date32, Date64, Time32, Time64, Ts32, Ts64, Ts96, Duration32, Duration64, Tz } =
  yggdryl.temporal
const { DataType } = yggdryl.types

test('the temporal namespace exposes the value types', () => {
  for (const cls of [Date32, Date64, Time32, Time64, Ts32, Ts64, Ts96, Duration32, Duration64, Tz]) {
    assert.equal(typeof cls, 'function')
  }
})

test('date calendar math and conversions', () => {
  const d = Date32.fromYmd(2024, 2, 29) // a leap day
  assert.deepEqual(d.toYmd(), [2024, 2, 29])
  assert.deepEqual([d.year, d.month, d.day], [2024, 2, 29])
  assert.ok(d.weekday() === 4 && d.isLeapYear())
  assert.equal(d.toString(), '2024-02-29')
  assert.ok(Date32.fromString('2024-02-29').equals(d))
  assert.deepEqual(Date32.fromDays(0).toYmd(), [1970, 1, 1])
  assert.throws(() => Date32.fromYmd(2023, 2, 29)) // not a leap year
  assert.deepEqual(d.toDate64().toYmd(), [2024, 2, 29])
  assert.ok(Date64.fromYmd(2024, 2, 29).toDate32().equals(d))
})

test('time components and units', () => {
  const t = Time32.fromHms(13, 45, 30)
  assert.deepEqual(t.toHms(), [13, 45, 30, 0])
  assert.ok(t.toString() === '13:45:30' && t.unit === 's')
  assert.equal(t.toUnit('ms').value, (13 * 3600 + 45 * 60 + 30) * 1000)
  const ns = Time64.fromHmsNano(1, 2, 3, 456000000)
  assert.equal(ns.toString(), '01:02:03.456000000')
  assert.deepEqual(Time64.fromString('01:02:03.456').toHms(), [1, 2, 3, 456000000])
})

test('timezone DST', () => {
  const paris = Tz.iana('Europe/Paris')
  assert.ok(paris.isIana() && paris.name === 'Europe/Paris')
  const winter = Ts64.fromDatetime(2024, 1, 15, 12, 0, 0, 0, 's', 'UTC')
  const summer = Ts64.fromDatetime(2024, 7, 15, 12, 0, 0, 0, 's', 'UTC')
  assert.equal(paris.offsetSecondsAt(winter.epochSeconds()), 3600) // CET
  assert.equal(paris.offsetSecondsAt(summer.epochSeconds()), 7200) // CEST
  assert.equal(Tz.parse('+02:00').offsetSecondsAt(0), 7200)
  assert.ok(Tz.parse('').isNaive())
  assert.throws(() => Tz.iana('Not/AZone'))
})

test('timestamp wall clock moves with the zone', () => {
  const utc = Ts64.fromDatetime(2024, 7, 15, 12, 0, 0, 0, 's', 'UTC')
  assert.deepEqual(utc.toDatetime(), [2024, 7, 15, 12, 0, 0, 0])
  const paris = utc.withTimezone('Europe/Paris')
  assert.deepEqual(paris.toDatetime(), [2024, 7, 15, 14, 0, 0, 0]) // +2h summer
  assert.equal(paris.epochValue, utc.epochValue) // same instant (bigint)
  assert.ok(paris.toString().endsWith('+02:00'))
  assert.deepEqual(utc.toDate().toYmd(), [2024, 7, 15])
  assert.equal(utc.toUnit('ms').epochValue, utc.epochValue * 1000n)
  assert.deepEqual(Ts64.fromString('2024-02-29T13:45:30Z').toDatetime(), [2024, 2, 29, 13, 45, 30, 0])
  // Ts96 holds a nanosecond count beyond i64's range.
  const far = Ts96.fromDatetime(5000, 1, 1, 0, 0, 0, 0, 'ns', 'UTC')
  assert.equal(far.year, 5000)
  assert.throws(() => Ts32.fromEpoch(10n ** 18n, 's', 'UTC'))
})

test('cross-type converters', () => {
  const date = Date32.fromYmd(2024, 2, 29)
  const time = Time64.fromHmsNano(13, 45, 30, 0)
  // Date <-> Timestamp (midnight, and at a wall-clock time).
  const midnight = date.atMidnight('s', 'UTC')
  assert.deepEqual(midnight.toDatetime(), [2024, 2, 29, 0, 0, 0, 0])
  assert.ok(midnight.toDate().equals(date))
  assert.deepEqual(date.atTime(time, 's', 'UTC').toDatetime(), [2024, 2, 29, 13, 45, 30, 0])
  // Date <-> Duration (days since epoch).
  assert.deepEqual([date.toDuration().value, date.toDuration().unit], [Number(date.days), 'd'])
  assert.ok(date.toDuration().toDate().equals(date))
  // Time <-> Duration, and Time -> Timestamp on the epoch date.
  assert.deepEqual(time.toDuration().toTime().toHms(), [13, 45, 30, 0])
  assert.deepEqual(time.toTimestamp('s', 'UTC').toDatetime(), [1970, 1, 1, 13, 45, 30, 0])
  // Timestamp <-> Duration.
  assert.equal(midnight.toDuration().toTimestamp('UTC').epochValue, midnight.epochValue)
  // Duration widths.
  assert.equal(Duration64.seconds(90).toDuration32().value, 90)
  assert.equal(Duration32.seconds(90).toDuration64().value, 90)
})

test('duration arithmetic', () => {
  const total = Duration64.seconds(1).add(Duration64.milliseconds(500))
  assert.deepEqual([total.value, total.unit], [1500, 'ms']) // aligns to the finer unit
  assert.equal(Duration64.seconds(90).toString(), '90s')
  assert.equal(Duration64.fromString('-1500ms').value, -1500)
  assert.equal(Duration64.seconds(1).compareTo(Duration64.milliseconds(500)), 1) // by span
  assert.equal(Duration64.seconds(5).neg().value, -5)
  assert.throws(() => Duration32.create(1, 'year')) // calendar unit unsupported
})

test('value identity and codec', () => {
  const values = [
    Date32.fromYmd(2024, 2, 29),
    Time64.fromHmsNano(1, 2, 3, 4),
    Ts64.fromDatetime(2024, 2, 29, 13, 45, 30, 0, 's', 'Europe/Paris'),
    Duration64.milliseconds(1234),
  ]
  for (const v of values) {
    assert.ok(v.equals(v.copy()))
    assert.equal(v.hashCode(), v.copy().hashCode())
    assert.ok(v.constructor.deserializeBytes(v.serializeBytes()).equals(v))
  }
})

test('generic parse factories and flexible formats', () => {
  const { date, time, timestamp, duration } = yggdryl.temporal
  for (const text of ['2024-02-29', '02/29/2024', '29.02.2024', 'Feb 29, 2024']) {
    assert.deepEqual(date(text).toYmd(), [2024, 2, 29], text)
  }
  assert.deepEqual(time('1:45 PM').toHms(), [13, 45, 0, 0]) // 12-hour
  const ts = timestamp('2024-02-29 13:45:30', 'ms', 'UTC')
  assert.deepEqual(ts.toDatetime(), [2024, 2, 29, 13, 45, 30, 0])
  assert.equal(ts.unit, 'ms')
  assert.equal(timestamp('2024-07-15T12:00:00-05:00').offsetSeconds(), -5 * 3600) // zone in string
  // Flexible durations: single-unit, compound, clock, and ISO-8601 — natural granularity.
  assert.equal(duration('90s').value, 90)
  assert.deepEqual([duration('1h30m').value, duration('1h30m').unit], [90, 'min'])
  assert.deepEqual([duration('1:30:00').value, duration('1:30:00').unit], [90, 'min'])
  assert.deepEqual([duration('PT1H30M').value, duration('PT1H30M').unit], [90, 'min'])
  assert.equal(duration('-1500ms').value, -1500)
  assert.deepEqual([duration('1h30m', 's').value, duration('1h30m', 's').unit], [5400, 's'])
})

test('JS Date bridge and signature', () => {
  const ts = Ts64.fromDatetime(2024, 2, 29, 13, 45, 30, 0, 'ms', 'UTC')
  const js = new Date(ts.toEpochMillis())
  assert.equal(js.toISOString(), '2024-02-29T13:45:30.000Z')
  assert.deepEqual(Ts64.fromEpochMillis(js.getTime()).toDatetime(), [2024, 2, 29, 13, 45, 30, 0])
  // Signature shows inner params + ISO value.
  assert.equal(Ts64.fromDatetime(2024, 2, 29, 13, 45, 30, 0, 's', 'UTC').signature(), 'ts64[s, UTC](2024-02-29T13:45:30Z)')
  assert.equal(Date32.fromYmd(2024, 2, 29).signature(), 'date32(2024-02-29)')
})

test('DataType knows temporals', () => {
  for (const [name, width] of [['date32', 4], ['time64', 8], ['ts96', 12], ['duration64', 8]]) {
    const dt = DataType.byName(name)
    assert.deepEqual([dt.name, dt.byteWidth, dt.category], [name, width, 'temporal'])
    assert.ok(dt.isTemporal() && !dt.isNumeric())
  }
  assert.ok(DataType.ts64().isTemporal())
})
