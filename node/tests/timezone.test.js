'use strict'

const assert = require('node:assert/strict')
const test = require('node:test')

const { Timezone } = require('yggdryl')

// Epoch seconds for a UTC civil moment, which is what every offset call takes.
function utc(year, month, day, hour = 0) {
  return Date.UTC(year, month - 1, day, hour) / 1000
}

// What the runtime's own database says the offset was, in seconds east of UTC.
function systemOffset(zone, epoch) {
  const parts = new Intl.DateTimeFormat('en-US', {
    hour12: false,
    timeZone: zone,
    timeZoneName: 'longOffset',
  }).formatToParts(new Date(epoch * 1000))
  const name = parts.find((part) => part.type === 'timeZoneName').value
  const match = /GMT([+-])(\d{2}):(\d{2})/.exec(name)
  if (match === null) return 0
  const magnitude = Number(match[2]) * 3600 + Number(match[3]) * 60
  return match[1] === '-' ? -magnitude : magnitude
}

test('a name, an alias, and a fixed offset all arrive at one value', () => {
  assert.ok(Timezone.fromString('Asia/Calcutta').equals(Timezone.from('Asia/Kolkata')))
  assert.equal(Timezone.from('US/Eastern').key, 'America/New_York')
  assert.equal(Timezone.fromOffset(5 * 3600 + 1800).key, '+05:30')
  assert.equal(Timezone.fromString('+0530').key, '+05:30')
  assert.equal(Timezone.fromOffset(0).key, 'UTC')

  // A zone read out of the runtime is a plain name, which is what parses.
  const resolved = Intl.DateTimeFormat().resolvedOptions().timeZone
  assert.equal(Timezone.from(resolved).key, Timezone.fromString(resolved).key)
  assert.ok(new Timezone(Timezone.from('UTC')).isUtc())
})

test('every spelling of UTC is UTC', () => {
  for (const value of ['UTC', 'utc', 'Z', 'GMT', 'Etc/UTC', '+00:00']) {
    assert.ok(Timezone.from(value).equals(Timezone.UTC))
    assert.ok(Timezone.from(value).isUtc())
  }
  assert.equal(Timezone.UTC.key, 'UTC')
  assert.ok(Timezone.UTC.isKnown())
  // UTC is the zero offset itself, so it is fixed where a place is not.
  assert.ok(Timezone.UTC.isFixed())
  assert.ok(Timezone.fromString('+05:30').isFixed())
  assert.ok(!Timezone.from('Europe/Paris').isFixed())
})

test('a bad name is refused rather than kept', () => {
  assert.throws(() => Timezone.fromString(''))
  assert.throws(() => Timezone.fromString('+25:00'))
  assert.throws(() => Timezone.fromOffset(25 * 3600))
  assert.throws(() => Timezone.fromOffset(30))
  assert.throws(() => Timezone.from(7))
})

test('a northern zone switches in March and November', () => {
  const newYork = Timezone.fromString('America/New_York')

  assert.equal(newYork.offsetAt(utc(2024, 1, 15)), -5 * 3600)
  assert.equal(newYork.offsetAt(utc(2024, 7, 15)), -4 * 3600)
  assert.equal(newYork.abbreviationAt(utc(2024, 1, 15)), 'EST')
  assert.equal(newYork.abbreviationAt(utc(2024, 7, 15)), 'EDT')
  assert.equal(newYork.isSavingAt(utc(2024, 7, 15)), true)
  assert.equal(newYork.isSavingAt(utc(2024, 1, 15)), false)
  assert.equal(newYork.standardOffset, -5 * 3600)
  assert.ok(newYork.observesSaving())
})

test('a southern zone is saving across the new year', () => {
  const sydney = Timezone.from('Australia/Sydney')

  assert.equal(sydney.offsetAt(utc(2024, 1, 15)), 11 * 3600)
  assert.equal(sydney.offsetAt(utc(2024, 7, 15)), 10 * 3600)
})

test('the offsets agree with the runtime time zone database', () => {
  // The registry is hand-rolled, so check it against Intl for the zones and
  // instants this build claims to know.
  for (const name of [
    'America/New_York',
    'America/Los_Angeles',
    'Europe/Paris',
    'Europe/London',
    'Australia/Sydney',
    'Asia/Tokyo',
    'Asia/Kolkata',
    'Pacific/Auckland',
  ]) {
    const zone = Timezone.from(name)
    for (const month of [1, 4, 7, 10]) {
      const epoch = utc(2024, month, 15, 12)
      assert.equal(zone.offsetAt(epoch), systemOffset(name, epoch), `${name} in month ${month}`)
    }
  }
})

test('local and UTC readings convert in both directions', () => {
  const paris = Timezone.from('Europe/Paris')
  const summer = utc(2024, 7, 15, 12)

  assert.equal(paris.intoLocal(summer), summer + 2 * 3600)
  assert.equal(paris.toLocal, undefined)
  assert.equal(paris.intoUtc(paris.intoLocal(summer)), summer)
  // The JavaScript spelling is minutes west, as `Date` reports it.
  assert.equal(paris.getTimezoneOffset(summer), -120)
  assert.equal(paris.getTimezoneOffset(utc(2024, 1, 15, 12)), -60)
  assert.throws(() => paris.offsetAt(1.5), /epoch/)
})

test('a zone without rules declines to answer', () => {
  const unknown = Timezone.fromString('Custom/Accepted')

  assert.equal(unknown.key, 'Custom/Accepted')
  assert.ok(!unknown.isKnown())
  assert.equal(unknown.offsetAt(0), null)
  assert.equal(unknown.standardOffset, null)
  assert.equal(unknown.abbreviationAt(0), null)
  assert.equal(unknown.isSavingAt(0), null)
  assert.equal(unknown.getTimezoneOffset(0), null)
  // Refusing is recoverable; a plausible wrong offset is not.
  assert.throws(() => unknown.intoLocal(0))
})

test('the registry needs no files, environment, or network', () => {
  const registered = Timezone.registered()
  const names = new Set(registered.map((zone) => zone.key))

  assert.ok(names.size > 60)
  for (const name of ['UTC', 'Europe/Paris', 'America/New_York', 'Asia/Tokyo']) {
    assert.ok(names.has(name), name)
  }
  // Every registered name parses back to itself.
  for (const zone of registered) {
    assert.ok(Timezone.from(zone.key).equals(zone))
  }

  for (const { alias, canonical } of Timezone.aliases()) {
    assert.equal(Timezone.from(alias).key, canonical)
  }
})

test('a zone behaves like the rest of the native value types', () => {
  const zone = Timezone.from('US/Pacific')

  assert.equal(zone.toString(), 'America/Los_Angeles')
  assert.equal(JSON.stringify({ zone }), '{"zone":"America/Los_Angeles"}')
  assert.equal(zone.stableHash(), Timezone.from('America/Los_Angeles').stableHash())
  assert.equal(typeof zone.stableHash(), 'bigint')
  assert.equal(zone.compare(zone.clone()), 0)
  assert.ok(zone.clone().equals(zone))
  assert.ok(!zone.equals(Timezone.UTC))

  const sorted = [Timezone.UTC, Timezone.from('Europe/Paris')]
    .sort((left, right) => left.compare(right))
    .map((value) => value.key)
  assert.deepEqual(sorted, ['Europe/Paris', 'UTC'])
})
