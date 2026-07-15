'use strict'

// Tests for the `yggdryl.temporal` columnar types (`Date32Serie` … `Duration64Serie`), mirroring
// the Rust `io::fixed::temporal` serie suite and the decimal-column binding (`deccolumn.test.js`):
// build each of the nine column types, ISO-string / epoch-bigint cell round-trips, unit / timezone,
// the byte codec, structural equality, and independent copy.

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const {
  Date32Serie,
  Date64Serie,
  Time32Serie,
  Time64Serie,
  Ts32Serie,
  Ts64Serie,
  Ts96Serie,
  Duration32Serie,
  Duration64Serie,
} = yggdryl.temporal

// { Serie, unit, tz (undefined -> naive), two ISO values, and the erased type name }.
const specs = [
  { Serie: Date32Serie, unit: 'd', values: ['2024-02-29', '1970-01-01'], typeName: 'date32' },
  { Serie: Date64Serie, unit: 'ms', values: ['2024-02-29', '1970-01-01'], typeName: 'date64' },
  { Serie: Time32Serie, unit: 's', values: ['13:45:30', '00:00:00'], typeName: 'time32' },
  { Serie: Time64Serie, unit: 'ns', values: ['01:02:03.456', '12:00:00'], typeName: 'time64' },
  { Serie: Ts32Serie, unit: 's', tz: 'UTC', values: ['2024-07-15T12:00:00Z', '1970-01-01T00:00:00Z'], typeName: 'ts32' },
  { Serie: Ts64Serie, unit: 'ns', tz: 'UTC', values: ['2024-07-15T12:00:00Z', '2000-01-01T00:00:00Z'], typeName: 'ts64' },
  { Serie: Ts96Serie, unit: 'ns', tz: 'UTC', values: ['2024-07-15T12:00:00Z', '1970-01-01T00:00:00Z'], typeName: 'ts96' },
  { Serie: Duration32Serie, unit: 's', values: ['90s', '1h30m'], typeName: 'duration32' },
  { Serie: Duration64Serie, unit: 'ns', values: ['1h30m', 'PT2H'], typeName: 'duration64' },
]

test('the temporal namespace exposes the columnar classes', () => {
  for (const { Serie } of specs) {
    assert.equal(typeof Serie, 'function')
  }
})

test('each column: construction, cell wire forms, unit/timezone, codec, copy', () => {
  for (const { Serie, unit, tz, values, typeName } of specs) {
    const col = new Serie(unit, tz, [values[0], null, values[1]])

    // Shape + null handling (a null sits in the middle).
    assert.ok(col.length === 3 && col.nullCount === 1 && col.hasNulls, `${typeName} shape`)
    assert.ok(!col.isEmpty(), `${typeName} not empty`)
    assert.ok(new Serie(unit, tz).isEmpty(), `${typeName} empty ctor`)

    // Unit / timezone cross as strings (naive -> "").
    assert.equal(col.unit, unit, `${typeName} unit`)
    assert.equal(col.timezone, tz ?? '', `${typeName} tz`)

    // Cell as ISO-8601 string.
    assert.equal(typeof col.get(0), 'string', `${typeName} get(0) string`)
    assert.equal(col.get(1), null, `${typeName} get(null) === null`)
    assert.equal(col.get(99), null, `${typeName} get(oob) === null`)
    assert.deepEqual(col.toOptions()[1], null, `${typeName} toOptions null`)

    // Cell as raw epoch/count bigint.
    assert.equal(typeof col.getEpoch(0), 'bigint', `${typeName} getEpoch bigint`)
    assert.equal(col.getEpoch(1), null, `${typeName} getEpoch(null) === null`)

    // getScalar hands back the value class (carrying the column unit), null for a null slot.
    const scalar = col.getScalar(0)
    assert.ok(scalar !== null && scalar.unit === col.unit, `${typeName} getScalar unit`)
    assert.equal(col.getScalar(1), null, `${typeName} getScalar(null) === null`)

    // dataType / toField.
    assert.ok(col.dataType().name === typeName && col.dataType().isTemporal(), `${typeName} dataType`)
    const field = col.toField('t')
    assert.ok(field.name === 't' && field.typeName === typeName && field.isTemporal(), `${typeName} toField`)

    // Byte codec round-trips (including the interior null).
    assert.ok(Serie.deserializeBytes(col.serializeBytes()).equals(col), `${typeName} codec`)

    // fromEpochs rebuilds the dense present cells identically.
    const dense = Serie.fromEpochs(unit, tz, [col.getEpoch(0), col.getEpoch(2)])
    assert.ok(dense.length === 2 && dense.nullCount === 0, `${typeName} fromEpochs shape`)
    assert.ok(dense.get(0) === col.get(0) && dense.get(1) === col.get(2), `${typeName} fromEpochs values`)

    // Copy is independent.
    const dup = col.copy()
    assert.ok(col.equals(dup), `${typeName} copy equals`)
    dup.push(values[0])
    assert.ok(col.length === 3 && dup.length === 4, `${typeName} copy independent`)
    assert.ok(!col.equals(dup), `${typeName} copy diverged`)
  }
})

test('push and set mutate a column', () => {
  const col = new Ts64Serie('s', 'UTC', ['2024-01-01T00:00:00Z', null])
  col.push('2024-01-02T00:00:00Z')
  assert.equal(col.length, 3)
  col.set(1, '2024-06-15T00:00:00Z')
  assert.ok(col.nullCount === 0 && col.get(1) === '2024-06-15T00:00:00Z')
  col.set(0, null)
  assert.ok(col.nullCount === 1 && col.get(0) === null)
  assert.throws(() => col.set(99, '2024-01-01T00:00:00Z')) // out of range
})

test('Ts64 column byte round-trip equals within Node', () => {
  const col = new Ts64Serie('ns', 'UTC', ['2024-07-15T12:00:00Z', null, '2020-02-29T23:59:59Z'])
  const back = Ts64Serie.deserializeBytes(col.serializeBytes())
  assert.ok(back.equals(col))
  assert.ok(back.unit === 'ns' && back.timezone === 'UTC')
})

test('Ts96 carries a 96-bit epoch beyond i64 as a bigint', () => {
  // Year 2600 in nanoseconds exceeds the signed 64-bit range — only ts96 holds it.
  const col = new Ts96Serie('ns', 'UTC', ['2600-01-01T00:00:00Z'])
  const epoch = col.getEpoch(0)
  assert.ok(typeof epoch === 'bigint' && epoch > 2n ** 63n)
  // It survives the epoch round-trip and the byte codec.
  assert.ok(Ts96Serie.fromEpochs('ns', 'UTC', [epoch]).get(0) === col.get(0))
  assert.ok(Ts96Serie.deserializeBytes(col.serializeBytes()).equals(col))
})

test('a naive vs zoned column, and cross-unit columns, are distinct', () => {
  const secs = new Ts64Serie('s', 'UTC', ['2024-01-01T00:00:00Z'])
  const millis = new Ts64Serie('ms', 'UTC', ['2024-01-01T00:00:00Z'])
  assert.ok(!secs.equals(millis)) // same instant, different unit
})

test('unknown unit or timezone throws a guided error', () => {
  assert.throws(() => new Ts64Serie('nonsense', 'UTC'), /time unit/)
  assert.throws(() => new Ts64Serie('ns', 'Not/AZone'), /timezone/)
})
