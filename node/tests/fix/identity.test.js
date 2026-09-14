'use strict'

// Settled clocks and named-content identity (decision 26): the default
// SendingTime a codec settles undated messages with, the four readers every
// message answers without a lookup - `updatedat`, `createdat`, `uuid`,
// `puuid` - and the replay fields no write, removal or row can take away.
//
// Every rule is the core's, pinned in `rust/tests/fix/content_identity.rs`;
// what these check is the crossing - a `Scalar` or a `Date` as the default,
// the located refusal each door throws, and a native row read back whole.

const assert = require('node:assert/strict')
const path = require('node:path')
const test = require('node:test')

const { DataType, Field, Scalar, fields, fix } = require('yggdryl')

const SEED = path.join(__dirname, '..', '..', '..', 'config', 'fix')

let seedRegistry
function seed() {
  seedRegistry ??= fix.FixRegistry.fromHandle(SEED)
  return seedRegistry.clone()
}

// The instant the Rust suite settles its hand-built messages at.
const CLOCK = 123_456_789n
// The replay bundle by tag, in the core's order.
const BUNDLE = [65003, 65023, 65017, 65018, 65024, 65025, 52]
const NIL = '00000000-0000-0000-0000-000000000000'

function clock(count) {
  return Scalar.datetime(BigInt(count), 'ns', 'UTC')
}

/** A message stating SendingTime at `CLOCK` and `extra` [field, value] pairs. */
function message(extra = [], registry = new fix.FixRegistry()) {
  const pairs = [[registry.fieldByTag(52), clock(CLOCK)], ...extra]
  const root = fields.struct('event', pairs.map(([field]) => field), { nullable: false })
  return new fix.FixMsg(root, pairs.map(([, value]) => value), registry)
}

function payload(value) {
  return [fields.utf8('payload', { nullable: true }), value]
}

function position(held, tag) {
  for (let at = 0; at < held.field.fieldLen; at += 1) {
    if (held.field.fieldAt(at).fix.tag === tag) return at
  }
  throw new Error(`no column for tag ${tag}`)
}

/** A located record refusal at `$.name`, as the core states one. */
function located(name) {
  return new RegExp(`invalid record value at \\$\\.${name}: expected .*, got `)
}

/** The four readers answer the row's own non-null cells, at their exact layouts. */
function assertMirrors(held) {
  for (const [tag, hard] of [
    [65003, held.updatedat()],
    [65023, held.createdat()],
    [65017, held.uuid()],
    [65018, held.puuid()],
  ]) {
    assert.ok(held.byTag(tag).equals(hard), `tag ${tag}`)
    assert.ok(held.value.at(position(held, tag)).equals(hard), `tag ${tag}`)
    assert.equal(held.field.fieldAt(position(held, tag)).nullable, false, `tag ${tag}`)
  }
  // The clocks hold nanoseconds in UTC exactly, never a restatement of them.
  for (const instant of [held.updatedat(), held.createdat()]) {
    assert.ok(instant.dtype.equals(clock(0).dtype), 'DateTime64(ns, UTC)')
  }
  assert.equal(held.uuid().id, 'uuid')
  assert.equal(held.puuid().id, 'uuid')
}

test('a fixed default sending time settles every clock and replays exactly', () => {
  const codec = new fix.FixCodec(new fix.FixRegistry(), { defaultSendingTime: clock(CLOCK) })
  assert.ok(codec.defaultSendingTime.equals(clock(CLOCK)))
  const bytes = Buffer.from('8=FIX.4.4|35=0|10=0|')
  const first = codec.parseLine(bytes).next().value
  const second = codec.parseLine(bytes).next().value
  assert.ok(first.equals(second))
  assert.ok(first.clone().equals(first))
  assert.ok(first.updatedat().equals(clock(CLOCK)))
  assert.ok(first.createdat().equals(clock(CLOCK)))
  assert.ok(first.byTag(65025).equals(clock(CLOCK)))
  assert.ok(first.byTag(52).equals(clock(CLOCK)))
  assert.equal(first.byTag(65024).asJs(), '')
  assert.ok(first.uuid().equals(second.uuid()))
  assert.deepEqual(first.intoBytes(124), bytes)
  assert.deepEqual(first.digest(), second.digest())
  assertMirrors(first)
})

test("the message's own SendingTime and TransactTime precede the default", () => {
  const codec = new fix.FixCodec(new fix.FixRegistry(), { defaultSendingTime: clock(CLOCK) })
  const held = codec.parseLine(
    Buffer.from('8=FIX.4.4|35=0|52=19700101-00:00:02.123456789|60=19700101-00:00:03.987654321|10=0|'),
  ).next().value
  assert.ok(held.byTag(52).equals(clock(2_123_456_789)))
  assert.ok(held.byTag(60).equals(clock(3_987_654_321)))
  assert.ok(held.byTag(65025).equals(clock(3_987_654_321)))
  assert.ok(held.updatedat().equals(clock(3_987_654_321)))
  assert.ok(held.createdat().equals(clock(3_987_654_321)))
  assertMirrors(held)
})

test('the default sending time crosses as a Scalar or a Date and is refused exactly', () => {
  const registry = new fix.FixRegistry()
  // Unstated or null, each new undated message reads UTC now.
  assert.equal(new fix.FixCodec(registry).defaultSendingTime, null)
  assert.equal(new fix.FixCodec(registry, { defaultSendingTime: null }).defaultSendingTime, null)
  // A Date is its UTC millisecond instant, restated in nanoseconds.
  const dated = new fix.FixCodec(registry, { defaultSendingTime: new Date(1_704_190_530_000) })
  assert.ok(dated.defaultSendingTime.equals(clock(1_704_190_530_000_000_000n)))
  assert.equal(dated.defaultSendingTime.count, 1_704_190_530_000_000_000n)
  assert.ok(dated.parseLine(Buffer.from('8=FIX.4.4|35=0|10=0|')).next().value.byTag(52).equals(dated.defaultSendingTime))
  // A Scalar crosses as it is, so any layout but nanoseconds in UTC is the
  // core's located refusal.
  assert.throws(
    () => new fix.FixCodec(registry, { defaultSendingTime: Scalar.datetime(0n, 'us', 'UTC') }),
    /\$\.default_sending_time/,
  )
  // Text is not a clock at all.
  assert.throws(() => new fix.FixCodec(registry, { defaultSendingTime: '1970-01-01T00:00:00Z' }))
})

test('explicitly stated event, grid and creation clocks are independent', () => {
  const registry = new fix.FixRegistry()
  const declared = (tag) => registry.fieldByTag(tag)
  const timestamp = new Field('timestamp', DataType.fromString('timestamp[ns, UTC]'), false)
  const held = message([
    [declared(60), clock(3)],
    [declared(65025), clock(9)],
    [declared(65003), clock(7)],
    [declared(65023), clock(6)],
    [timestamp, clock(999)],
  ], registry)
  assert.ok(held.updatedat().equals(clock(7)))
  assert.ok(held.createdat().equals(clock(6)))
  assert.ok(held.byTag(65025).equals(clock(9)))
  // A capture's `timestamp` is ordinary context, never a FIX clock.
  assert.ok(held.byName('timestamp').equals(clock(999)))
  assert.equal(held.field.fieldAt(held.field.indexOf('timestamp')).fix.tag, null)
  assertMirrors(held)
})

test('puuid hashes only the exact code bytes, the empty name included', () => {
  const registry = new fix.FixRegistry()
  const identities = new Set()
  for (const code of ['', ' ', 'alpha', 'alpha ', 'é']) {
    const held = message([[registry.fieldByTag(65024), code]], registry)
    const expected = held.puuid()
    assert.notEqual(expected.asJs(), NIL)
    held.set(65003, clock(-1))
    held.set(7777, 'different content')
    assert.ok(held.puuid().equals(expected), JSON.stringify(code))
    // Every message naming the same code answers the same puuid.
    assert.ok(message([payload('other'), [registry.fieldByTag(65024), code]], registry).puuid().equals(expected))
    identities.add(expected.asJs())
  }
  assert.equal(identities.size, 5)
})

test('named content ignores root order and metadata, never names or nulls', () => {
  const pairs = () => [
    payload('value'),
    [fields.int32('Z', { nullable: false }), 7],
    [fields.utf8('é', { nullable: false }), 'text'],
  ]
  const first = message(pairs())
  const reversed = pairs().reverse()
  for (const [field] of reversed) field.set('example:note', 'not content')
  const second = message(reversed)
  assert.ok(first.uuid().equals(second.uuid()))
  assert.equal(first.equals(second), false, 'message equality still includes the schema')
  const absent = message()
  const nulled = message([payload(null)])
  const empty = message([payload('')])
  const renamed = message([[fields.utf8('renamed', { nullable: true }), null]])
  assert.equal(new Set([absent, nulled, empty, renamed].map((held) => held.uuid().asJs())).size, 4)
})

test('ordinary mutation recomputes identity and excludes only the owned clocks', () => {
  const held = message([payload('first')])
  const before = held.clone()
  held.set(65023, clock(-10))
  assert.ok(held.uuid().equals(before.uuid()), 'creation time is excluded')
  assert.equal(held.equals(before), false)
  held.set(65003, clock(CLOCK + 1n))
  assert.equal(held.uuid().equals(before.uuid()), false, 'one nanosecond remains visible')
  let old = held.uuid()
  held.set('payload', 'second')
  assert.equal(held.uuid().equals(old), false)
  const settled = held.updatedat()
  old = held.uuid()
  held.set(52, clock(CLOCK + 2n))
  assert.equal(held.uuid().equals(old), false)
  assert.ok(held.updatedat().equals(settled), 'mutating SendingTime does not reread or reset clocks')
  old = held.uuid()
  held.set(65025, clock(CLOCK + 3n))
  assert.equal(held.uuid().equals(old), false)
  assert.ok(held.puuid().equals(before.puuid()))
  assertMirrors(held)
})

test('an explicit identity write asserts the complete candidate atomically', () => {
  const held = message([payload('first')])
  const before = held.clone()
  for (const [tag, name] of [[65017, 'uuid'], [65018, 'puuid']]) {
    assert.throws(() => held.set(tag, NIL), located(name))
    assert.ok(held.equals(before))
  }
  // A write naming another code moves the chain identity with it, and stating
  // the identity the new content computes is accepted.
  const expected = held.clone()
  expected.set(65024, 'chain')
  assert.equal(expected.puuid().equals(before.puuid()), false)
  const asserted = expected.clone()
  asserted.set(65018, expected.puuid())
  assert.ok(asserted.equals(expected))
  assertMirrors(asserted)
})

test('mandatory null writes and removals refuse without changing any state', () => {
  const held = message([payload('first')])
  for (const tag of BUNDLE) {
    const before = held.clone()
    const name = held.field.fieldAt(position(held, tag)).name
    assert.throws(() => held.set(tag, null), located(name))
    assert.ok(held.equals(before), name)
    assert.throws(() => held.remove(tag), located(name))
    assert.ok(held.equals(before), name)
    assertMirrors(held)
  }
  const old = held.uuid()
  assert.equal(held.remove('payload').asJs(), 'first')
  assert.equal(held.uuid().equals(old), false)
  const before = held.clone()
  assert.equal(held.remove('absent'), null)
  assert.ok(held.equals(before))
})

test('the replay bundle is required before record defaults can supply a value', () => {
  const original = message([payload('value')])
  const cells = () => Array.from({ length: original.field.fieldLen }, (_, at) => original.value.at(at))
  const members = () => Array.from({ length: original.field.fieldLen }, (_, at) => original.field.fieldAt(at))
  for (const tag of BUNDLE) {
    const at = position(original, tag)
    const name = original.field.fieldAt(at).name
    // A schema without the field neither exports nor replays a row.
    const narrowMembers = members()
    narrowMembers.splice(at, 1)
    const narrow = fields.struct('event', narrowMembers, { nullable: false })
    const narrowCells = cells()
    narrowCells.splice(at, 1)
    assert.throws(() => original.intoRow(narrow), located(name))
    assert.throws(() => fix.FixMsg.fromRow(narrow, narrowCells, original.registry), located(name))
    // A nullable declaration of it is refused too.
    const loose = members()
    loose[at] = loose[at].clone()
    loose[at].setNullable(true)
    assert.throws(
      () => fix.FixMsg.fromRow(fields.struct('event', loose, { nullable: false }), original.value, original.registry),
      located(name),
    )
    // And so is a null or missing value under the settled schema.
    const nulled = cells()
    nulled[at] = null
    assert.throws(() => fix.FixMsg.fromRow(original.field, nulled, original.registry), located(name))
  }
})

test('replay refuses tampered identity and non-native mandatory values', () => {
  const original = message()
  const other = message([payload('other')])
  for (const tag of BUNDLE) {
    const at = position(original, tag)
    const cells = Array.from({ length: original.field.fieldLen }, (_, index) => original.value.at(index))
    // A native UUID another message computed is a tampered identity; an
    // integer is no code; a microsecond instant is no replay clock.
    cells[at] = tag === 65017 || tag === 65018
      ? other.uuid()
      : tag === 65024
        ? 7
        : Scalar.datetime(0n, 'us', 'UTC')
    assert.throws(
      () => fix.FixMsg.fromRow(original.field, cells, original.registry),
      located(original.field.fieldAt(at).name),
    )
  }
})

test('a full projection preserves identity and a lossy one stabilizes on the second export', () => {
  const original = message([payload('value')])
  const full = original.intoRow(original.field)
  assert.ok(full.equals(original.value))
  const restored = fix.FixMsg.fromRow(original.field, full, original.registry)
  assert.ok(restored.equals(original))

  const narrowMembers = []
  for (let at = 0; at < original.field.fieldLen; at += 1) {
    if (original.field.fieldAt(at).name !== 'payload') narrowMembers.push(original.field.fieldAt(at))
  }
  const narrow = fields.struct('event', narrowMembers, { nullable: false })
  const first = original.intoRow(narrow)
  const held = fix.FixMsg.fromRow(narrow, first, original.registry)
  assert.equal(held.uuid().equals(original.uuid()), false)
  assert.ok(held.puuid().equals(original.puuid()))
  assert.ok(held.intoRow(narrow).equals(first))
  assert.ok(original.intoRow(original.field).equals(full), 'source unchanged')

  const paddedMembers = [...Array(original.field.fieldLen).keys()].map((at) => original.field.fieldAt(at))
  paddedMembers.push(fields.utf8('padding', { nullable: true }))
  const padded = fields.struct('event', paddedMembers, { nullable: false })
  const once = original.intoRow(padded)
  const heldPadded = fix.FixMsg.fromRow(padded, once, original.registry)
  assert.equal(heldPadded.uuid().equals(original.uuid()), false)
  assert.ok(heldPadded.intoRow(padded).equals(once))
})

test('enrichment keeps the settled clocks and the arrival record, and finalizes once', () => {
  const codec = new fix.FixCodec(seed(), { defaultSendingTime: clock(CLOCK) })
  const raw = codec.parseLine(Buffer.from('8=FIX.4.4|35=D|11=A1|55=ALPHA|54=1|10=0|')).next().value
  const enriched = codec.enrichMessage(raw)
  assert.ok(enriched.updatedat().equals(raw.updatedat()))
  assert.ok(enriched.createdat().equals(raw.createdat()))
  assert.ok(enriched.byTag(52).equals(raw.byTag(52)))
  assert.deepEqual(enriched.arrivals(), raw.arrivals())
  assert.deepEqual(enriched.intoBytes(124), raw.intoBytes(124))
  assert.deepEqual(enriched.digest(), raw.digest())
  assert.ok(codec.enrichMessage(enriched).equals(enriched))
  assertMirrors(enriched)
})
