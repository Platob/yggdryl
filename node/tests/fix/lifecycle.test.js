'use strict'

// The lifecycle's cadence: one normalized transition behind `fill`, the
// filtered `snapshot` door and the `snapshots` stream, epoch-grid buckets of
// `intervalNs`, and the first creation instant a live chain carries.
//
// Every rule is the core's, pinned in `rust/tests/fix/lifecycle_grid.rs`
// (decisions 26 and 27); what these check is the crossing - the interval as a
// `bigint` or an exact number, the refusals that arrive located, the stream a
// JavaScript iterable feeds one message at a time, and the lifecycle that
// stream owns once it is answered.

const assert = require('node:assert/strict')
const test = require('node:test')

const { BatchReader, DataType, Scalar, fields, fix } = require('yggdryl')

const I64_MIN = -(2n ** 63n)
const I64_MAX = 2n ** 63n - 1n

const UPDATEDAT = 65003
const INSTUUID = 65016
const STATE = 65015
const ALTIDS = 65020
const PREVUPDATEDAT = 65021
const PREVUUID = 65022
const CREATEDAT = 65023
const CODE = 65024
const SNAPSHOTAT = 65025

/** A nanosecond UTC instant, the one layout every FIX clock holds. */
function clock(nanos) {
  return Scalar.datetime(BigInt(nanos), 'ns', 'UTC')
}

/**
 * The sixteen identity bytes the Rust suite's `numbered_identity(value)`
 * builds: the number big-endian, as the `Buffer` a `fixedbinary(16)` column
 * takes.
 */
function identityOf(value) {
  return Buffer.from(value.toString(16).padStart(32, '0'), 'hex')
}

/**
 * One event every clock of which is `nanos`, named by `code`, in an optional
 * instrument `scope`, stating `identifiers` as its altids Map - the Rust
 * suite's `event` helper, built through the public constructor.
 */
function event(registry, nanos, code, scope = null, identifiers = []) {
  const members = [52, UPDATEDAT, CREATEDAT, SNAPSHOTAT, CODE].map((tag) => registry.fieldByTag(tag))
  members.push(registry.groupByTag(ALTIDS))
  const values = [clock(nanos), clock(nanos), clock(nanos), clock(nanos), code, new Map(identifiers)]
  if (scope !== null) {
    members.push(registry.fieldByTag(INSTUUID))
    values.push(scope)
  }
  return new fix.FixMsg(fields.struct('event', members, { nullable: false }), values, registry)
}

/** A copy of `message` with one more value, as Rust's consuming `with_value`. */
function withValue(message, key, value) {
  const copy = message.clone()
  copy.set(key, value)
  return copy
}

/** A copy of `message` in a terminal order state. */
function terminal(message) {
  return withValue(message, STATE, 'Filled')
}

function lifecycle(registry, intervalNs = 10n) {
  return new fix.FixLifecycle(registry, { intervalNs })
}

/** The previous stamps `message` carries: `expected`'s grid clock and UUID, or null. */
function previous(message, expected) {
  const held = [message.byTag(PREVUPDATEDAT), message.byTag(PREVUUID)]
  if (expected === null) {
    assert.deepEqual(held.map((value) => value.kind), ['null', 'null'])
    return
  }
  assert.ok(held[0].equals(expected.updatedat()), 'prevupdatedat is the previous grid instant')
  assert.ok(held[1].equals(expected.uuid()), 'prevuuid is the previous message identity')
}

/** Whether a snapshot answer is `expected`, or no answer where none is expected. */
function snapshotIs(answer, expected) {
  if (expected === null) {
    assert.equal(answer, null)
    return
  }
  assert.notEqual(answer, null)
  assert.ok(answer.equals(expected))
}

/** The one puuid any message naming `code` answers: the hash of the code alone. */
function persistentOf(code) {
  return event(new fix.FixRegistry(), 0, code).puuid()
}

/** A registry whose previous-message field at `tag` is declared under `dtype`. */
function wrongPrevious(tag, dtype) {
  const registry = new fix.FixRegistry()
  const field = registry.remove(tag)
  field.setDtype(dtype)
  field.setNullable(true)
  registry.insert(field)
  return registry
}

test('the cadence is positive, atomic and retained by clear', () => {
  const registry = new fix.FixRegistry()
  const life = new fix.FixLifecycle(registry)
  assert.equal(fix.FixLifecycle.DEFAULT_INTERVAL_NS, 1_000_000_000n)
  assert.equal(typeof life.intervalNs, 'bigint')
  assert.equal(life.intervalNs, fix.FixLifecycle.DEFAULT_INTERVAL_NS)
  // The constant is the core's, read once: it cannot be reassigned.
  assert.throws(() => {
    fix.FixLifecycle.DEFAULT_INTERVAL_NS = 1n
  }, TypeError)
  for (const invalid of [0n, -1n, I64_MIN]) {
    assert.throws(() => life.setIntervalNs(invalid), /\$\.interval_ns/)
    assert.equal(life.intervalNs, fix.FixLifecycle.DEFAULT_INTERVAL_NS)
    assert.equal(life.alive, 0)
  }
  // An exact number crosses as the same count a bigint does.
  life.setIntervalNs(10)
  assert.equal(life.intervalNs, 10n)
  life.fill(event(registry, 1, 'A'))
  // Repeating the current interval is a no-op even while a chain is live;
  // any change then is refused and changes nothing.
  life.setIntervalNs(10n)
  for (const invalid of [0, -1, 20]) {
    assert.throws(() => life.setIntervalNs(invalid), /\$\.interval_ns/)
    assert.equal(life.intervalNs, 10n)
    assert.equal(life.alive, 1)
  }
  life.clear()
  assert.equal(life.intervalNs, 10n)
  assert.equal(life.alive, 0)
  life.setIntervalNs(I64_MAX)
  assert.equal(life.intervalNs, I64_MAX)
  assert.throws(() => new fix.FixLifecycle(registry, { intervalNs: 0 }), /\$\.interval_ns/)
  assert.equal(new fix.FixLifecycle(registry, {}).intervalNs, fix.FixLifecycle.DEFAULT_INTERVAL_NS)
  // The boundary refuses what is not one exact signed 64-bit count before the
  // core sees it.
  assert.throws(() => life.setIntervalNs(1.5), /intervalNs/)
  assert.throws(() => life.setIntervalNs(2n ** 63n), /intervalNs must fit a signed 64-bit integer/)
  assert.throws(() => new fix.FixLifecycle(registry, { intervalNs: 2n ** 63n }), /intervalNs/)
  assert.equal(life.intervalNs, I64_MAX)
})

test('the epoch floor uses negative buckets and a boundary opens its bucket', () => {
  const registry = new fix.FixRegistry()
  for (const [time, interval, grid] of [
    [-11n, 10n, -20n],
    [-10n, 10n, -10n],
    [-1n, 10n, -10n],
    [0n, 10n, 0n],
    [9n, 10n, 0n],
    [10n, 10n, 10n],
    [11n, 10n, 10n],
    [I64_MIN, 1n, I64_MIN],
    [I64_MAX, 1n, I64_MAX],
    [I64_MAX, I64_MAX, I64_MAX],
  ]) {
    const raw = event(registry, time, 'A')
    const filled = lifecycle(registry, interval).fill(raw)
    assert.ok(filled.updatedat().equals(clock(grid)), `${time} at ${interval}`)
    assert.ok(filled.createdat().equals(clock(time)), `${time} at ${interval}`)
    assert.ok(filled.byTag(SNAPSHOTAT).equals(clock(time)), `${time} at ${interval}`)
    // Only an arrival off the grid is a new snapshot.
    snapshotIs(lifecycle(registry, interval).snapshot(raw), time !== grid ? filled : null)
  }
})

test("creation is the first arrival's statement, not the minimum or the grid", () => {
  const registry = new fix.FixRegistry()
  const life = lifecycle(registry)
  const otherCreation = lifecycle(registry)
  let last = null
  for (const [time, grid, stated] of [[21, 20, 987], [1, 0, -123], [31, 30, 432]]) {
    const raw = withValue(event(registry, time, 'A'), CREATEDAT, clock(stated))
    const comparison = otherCreation.fill(withValue(raw, CREATEDAT, clock(654)))
    const filled = life.fill(raw)
    assert.ok(filled.createdat().equals(clock(987)))
    assert.ok(comparison.createdat().equals(clock(654)))
    assert.ok(filled.updatedat().equals(clock(grid)))
    assert.ok(filled.byTag(SNAPSHOTAT).equals(clock(time)))
    // Creation is not content: the identities agree whatever it says.
    assert.ok(filled.uuid().equals(comparison.uuid()))
    assert.ok(filled.puuid().equals(comparison.puuid()))
    previous(filled, last)
    last = filled
  }
})

test('the full and filtered doors share finalized history and consume aligned buckets', () => {
  const registry = new fix.FixRegistry()
  const full = lifecycle(registry)
  const filtered = lifecycle(registry)
  let last = null
  for (const [time, grid, emit] of [[1, 0, true], [7, 0, false], [10, 10, false], [11, 10, false], [21, 20, true]]) {
    const raw = event(registry, time, 'A')
    const filled = full.fill(raw)
    previous(filled, last)
    assert.ok(filled.updatedat().equals(clock(grid)), `${time}`)
    assert.ok(filled.createdat().equals(clock(1)), `${time}`)
    assert.ok(filled.byTag(SNAPSHOTAT).equals(clock(time)), `${time}`)
    snapshotIs(filtered.snapshot(raw), emit ? filled : null)
    last = filled
  }
  assert.equal(full.alive, 1)
  assert.equal(filtered.alive, 1)

  // An aligned first arrival consumes its bucket without emitting, and still
  // establishes the chain's creation.
  const aligned = lifecycle(registry)
  assert.equal(aligned.snapshot(withValue(event(registry, 10, 'B'), CREATEDAT, clock(77))), null)
  assert.equal(aligned.snapshot(event(registry, 19, 'B')), null)
  const afterAligned = aligned.snapshot(event(registry, 21, 'B'))
  assert.notEqual(afterAligned, null)
  assert.ok(afterAligned.createdat().equals(clock(77)))
})

test('explicit codes are global and never steal scoped identifier ownership', () => {
  const registry = new fix.FixRegistry()
  const life = lifecycle(registry)
  const scope = identityOf(1)
  const first = life.fill(event(registry, 1, 'A', scope, [['id', 'OWNED']]))
  const other = life.fill(event(registry, 2, 'B', scope, [['id', 'OWNED']]))
  assert.equal(first.puuid().equals(other.puuid()), false)
  assert.ok(first.createdat().equals(clock(1)))
  assert.ok(other.createdat().equals(clock(2)))
  previous(other, null)
  assert.equal(life.alive, 2)
  // An unnamed message reaching the owned identifier joins its owner.
  const alias = life.fill(event(registry, 11, '', scope, [['id', 'OWNED']]))
  assert.equal(alias.byTag(CODE).asJs(), 'A')
  previous(alias, first)
  assert.ok(alias.createdat().equals(first.createdat()))

  // An explicit code joins its chain across instrument scopes.
  const direct = life.fill(event(registry, 21, 'A', identityOf(2), [['id', 'NEW']]))
  previous(direct, alias)
  assert.ok(direct.puuid().equals(first.puuid()))
  assert.ok(direct.createdat().equals(first.createdat()))
  const attached = life.fill(event(registry, 31, '', identityOf(2), [['id', 'NEW']]))
  previous(attached, direct)
  assert.ok(attached.createdat().equals(first.createdat()))
  assert.equal(life.alive, 2)

  // When two identifiers reach different chains, canonical member name order
  // wins, and the other owner keeps its key.
  const b = life.fill(event(registry, 41, 'B', scope, [['id', 'OTHER']]))
  const joined = life.fill(event(registry, 51, '', scope, [['a', 'OTHER'], ['z', 'OWNED']]))
  previous(joined, b)
  assert.ok(joined.puuid().equals(other.puuid()))
  assert.ok(b.createdat().equals(other.createdat()))
  assert.ok(joined.createdat().equals(other.createdat()))
  const stillA = life.fill(event(registry, 61, '', scope, [['id', 'OWNED']]))
  previous(stillA, attached)
  assert.ok(stillA.createdat().equals(first.createdat()))
})

test('derived codes keep the scope and identifier text, and empty is not whitespace', () => {
  const registry = new fix.FixRegistry()
  const life = lifecycle(registry)
  const text = 'Mixed/Case/界'
  const scopes = [null, identityOf(0), identityOf(1)]
  const ids = []
  for (const [at, scope] of scopes.entries()) {
    const time = at + 1
    const expected = scope === null ? `-/${text}` : `${scope.toString('hex')}/${text}`
    const value = life.fill(event(registry, time, '', scope, [['id', text]]))
    assert.equal(value.byTag(CODE).asJs(), expected)
    assert.ok(value.puuid().equals(persistentOf(expected)))
    previous(value, null)
    assert.ok(value.createdat().equals(clock(time)))
    assert.ok(ids.every((held) => !held.equals(value.puuid())))
    ids.push(value.puuid())
  }
  assert.equal(life.alive, 3)
  for (const [at, scope] of scopes.entries()) {
    const joined = life.fill(event(registry, 31, '', scope, [['id', text]]))
    assert.ok(joined.createdat().equals(clock(at + 1)))
    assert.ok(joined.puuid().equals(ids[at]))
  }

  // No name opens no chain and emits no snapshot; its puuid is still the one
  // deterministic hash of the empty code.
  const unknown = event(registry, 1, '')
  const filled = life.fill(unknown)
  assert.ok(filled.puuid().equals(persistentOf('')))
  assert.ok(filled.updatedat().equals(clock(0)))
  assert.ok(filled.createdat().equals(unknown.createdat()))
  previous(filled, null)
  assert.equal(life.snapshot(unknown), null)
  const nextUnnamed = life.fill(event(registry, 2, ''))
  assert.ok(nextUnnamed.createdat().equals(clock(2)))
  previous(nextUnnamed, null)
  assert.equal(life.alive, 3)
  // Whitespace is a real name.
  const whitespace = life.snapshot(event(registry, 1, ' '))
  assert.notEqual(whitespace, null)
  assert.equal(whitespace.byTag(CODE).asJs(), ' ')
  assert.equal(life.alive, 4)
})

test('late messages advance history without lowering the bucket high-water mark', () => {
  const registry = new fix.FixRegistry()
  const full = lifecycle(registry)
  const filtered = lifecycle(registry)
  let last = null
  for (const [time, emit] of [[21, true], [1, false], [29, false], [31, true]]) {
    const raw = event(registry, time, 'A')
    const filled = full.fill(raw)
    previous(filled, last)
    assert.ok(filled.createdat().equals(clock(21)))
    assert.ok(filled.byTag(SNAPSHOTAT).equals(clock(time)))
    snapshotIs(filtered.snapshot(raw), emit ? filled : null)
    last = filled
  }
})

test('a suppressed terminal closes and a same-bucket reopening starts fresh', () => {
  const registry = new fix.FixRegistry()
  const life = lifecycle(registry)
  const full = lifecycle(registry)
  const raw = event(registry, 1, 'A', null, [['id', 'OLD']])
  const first = life.snapshot(raw)
  assert.notEqual(first, null)
  assert.ok(full.fill(raw).equals(first))
  const ending = terminal(event(registry, 2, 'A'))
  const closed = full.fill(ending)
  assert.ok(closed.createdat().equals(first.createdat()))
  previous(closed, first)
  assert.equal(full.alive, 0)
  assert.equal(life.snapshot(ending), null)
  assert.equal(life.alive, 0)
  const reopened = life.snapshot(event(registry, 3, 'A'))
  assert.notEqual(reopened, null)
  previous(reopened, null)
  assert.ok(reopened.puuid().equals(first.puuid()))
  assert.ok(reopened.createdat().equals(clock(3)))
  // The closed chain's identifier is free again, for a chain of its own.
  const oldKey = life.fill(event(registry, 4, '', null, [['id', 'OLD']]))
  assert.equal(oldKey.puuid().equals(reopened.puuid()), false)
  previous(oldKey, null)
  assert.ok(oldKey.createdat().equals(clock(4)))
  assert.equal(life.alive, 2)
  life.clear()
  assert.equal(life.intervalNs, 10n)
  for (const time of [5, 6]) {
    const standalone = life.snapshot(terminal(event(registry, time, 'A')))
    assert.notEqual(standalone, null)
    previous(standalone, null)
    assert.ok(standalone.createdat().equals(clock(time)))
    assert.equal(life.alive, 0, 'no closed-chain tombstones')
  }
  assert.equal(life.snapshot(terminal(event(registry, 10, 'A'))), null)
  assert.equal(life.alive, 0)
})

test('grid underflow refuses before opening, attaching, advancing or closing', () => {
  const registry = new fix.FixRegistry()
  for (const existing of [false, true]) {
    const life = lifecycle(registry)
    const first = existing ? life.fill(event(registry, 1, 'A', null, [['id', 'LIVE']])) : null
    const failed = terminal(event(registry, I64_MIN, 'A', null, [['id', 'NEW']]))
    assert.throws(() => life.snapshot(failed), /\$\.updatedat/)
    assert.equal(life.alive, existing ? 1 : 0)
    const accepted = life.snapshot(event(registry, 11, 'A'))
    assert.notEqual(accepted, null)
    previous(accepted, first)
    assert.ok(accepted.createdat().equals(clock(existing ? 1 : 11)))
    const free = life.fill(event(registry, 12, '', null, [['id', 'NEW']]))
    previous(free, null)
    assert.ok(free.createdat().equals(clock(12)))
    assert.equal(free.puuid().equals(accepted.puuid()), false)
  }
})

test('a previous stamp its target cannot hold consumes no bucket, closes and attaches nothing', () => {
  const base = new fix.FixRegistry()
  for (const [tag, name, dtype] of [
    [PREVUUID, 'prevuuid', DataType.from('utf8')],
    [PREVUUID, 'prevuuid', DataType.from('binary')],
    [PREVUUID, 'prevuuid', clock(0).dtype],
    [PREVUPDATEDAT, 'prevupdatedat', DataType.from('uuid')],
    [PREVUPDATEDAT, 'prevupdatedat', DataType.from('utf8')],
  ]) {
    const custom = wrongPrevious(tag, dtype)
    for (const existing of [false, true]) {
      const life = lifecycle(base)
      const first = existing ? life.fill(event(base, 1, 'A', null, [['id', 'LIVE']])) : null
      const failed = withValue(terminal(event(custom, 11, 'A', null, [['a', 'LIVE'], ['b', 'NEW']])), tag, null)
      assert.throws(() => life.snapshot(failed), new RegExp(`\\$\\.${name}`))
      assert.equal(life.alive, existing ? 1 : 0)
      const accepted = life.snapshot(event(base, 12, 'A'))
      assert.notEqual(accepted, null)
      previous(accepted, first)
      assert.ok(accepted.createdat().equals(clock(existing ? 1 : 12)))
      const free = life.fill(event(base, 13, '', null, [['id', 'NEW']]))
      previous(free, null)
      assert.ok(free.createdat().equals(clock(13)))
      assert.equal(free.puuid().equals(accepted.puuid()), false)
    }
  }
})

test('independently stated previous values are not the current history', () => {
  const registry = new fix.FixRegistry()
  const statedIdentity = identityOf(987)
  for (const [statedClock, statedId] of [[false, false], [true, false], [false, true], [true, true]]) {
    const life = lifecycle(registry)
    const first = life.fill(event(registry, 1, 'A'))
    let second = event(registry, 11, 'A')
    second = withValue(second, PREVUPDATEDAT, statedClock ? clock(987) : null)
    second = withValue(second, PREVUUID, statedId ? statedIdentity : null)
    second = life.fill(second)
    assert.ok(second.byTag(PREVUPDATEDAT).equals(statedClock ? clock(987) : first.updatedat()))
    if (statedId) {
      assert.deepEqual(Buffer.from(second.byTag(PREVUUID).asJs()), statedIdentity)
    } else {
      assert.ok(second.byTag(PREVUUID).equals(first.uuid()))
    }
    // The stored pair is the current message's own, never what it stated.
    previous(life.fill(event(registry, 21, 'A')), second)
  }
})

test('the snapshot stream is lazy, throws a refused message where it is met and fuses', () => {
  const registry = new fix.FixRegistry()
  const custom = wrongPrevious(PREVUUID, DataType.from('utf8'))
  const bad = withValue(event(custom, 11, 'A'), PREVUUID, null)
  const input = [
    event(registry, 1, 'A'),
    bad,
    event(registry, 12, 'A'),
    event(registry, 20, 'A'),
    event(registry, 29, 'A'),
    event(registry, 31, 'A'),
  ]
  let pulls = 0
  const source = {
    [Symbol.iterator]() {
      return {
        next() {
          pulls += 1
          return pulls <= input.length ? { value: input[pulls - 1], done: false } : { value: undefined, done: true }
        },
      }
    },
  }
  const snapshots = lifecycle(registry).snapshots(source)
  assert.ok(snapshots instanceof fix.FixMessages)
  assert.equal(snapshots[Symbol.iterator](), snapshots)
  assert.equal(pulls, 0)
  const first = snapshots.next().value
  assert.equal(pulls, 1)
  // A transition the core refuses throws where it is met, advances nothing,
  // and the stream goes on.
  assert.throws(() => snapshots.next(), /\$\.prevuuid/)
  assert.equal(pulls, 2)
  const recovered = snapshots.next().value
  previous(recovered, first)
  assert.ok(recovered.createdat().equals(first.createdat()))
  assert.equal(pulls, 3, 'the refused transition did not consume bucket ten')
  // Aligned and equal buckets are processed, not emitted.
  assert.ok(snapshots.next().value.updatedat().equals(clock(30)))
  assert.equal(pulls, 6)
  assert.equal(snapshots.next().done, true)
  assert.equal(pulls, 7)
  assert.equal(snapshots.next().done, true)
  assert.equal(pulls, 7, 'exhaustion is fused')

  // A failure of the iterable throws as itself, once, and ends the stream.
  function* failing() {
    yield event(registry, 1, 'LATE')
    throw new RangeError('the source broke')
  }
  const broken = lifecycle(registry).snapshots(failing())
  assert.notEqual(broken.next().value, undefined)
  assert.throws(() => broken.next(), RangeError)
  assert.equal(broken.next().done, true)
  // What is not iterable is refused before anything is pulled, and an item
  // that is not a message where it is met.
  assert.throws(() => lifecycle(registry).snapshots(42), TypeError)
  const mixed = lifecycle(registry).snapshots([event(registry, 1, 'A'), '8=FIX.4.4|35=0|10=0|'])
  assert.equal(mixed.next().done, false)
  assert.throws(() => mixed.next(), TypeError)
  assert.equal(mixed.next().done, true)
})

test('the stream snapshots answers owns its lifecycle', () => {
  const registry = new fix.FixRegistry()
  const life = new fix.FixLifecycle(registry, { intervalNs: 10n })
  life.fill(event(registry, 1, 'A'))
  // The stream takes the configured interval and the live chain with it.
  const stream = life.snapshots([event(registry, 11, 'A')])
  const owned = /owned by the stream snapshots\(\) answered/
  assert.throws(() => life.fill(event(registry, 21, 'A')), owned)
  assert.throws(() => life.snapshot(event(registry, 21, 'A')), owned)
  assert.throws(() => life.alive, owned)
  assert.throws(() => life.intervalNs, owned)
  assert.throws(() => life.setIntervalNs(10n), owned)
  assert.throws(() => life.clear(), owned)
  assert.throws(() => life.snapshots([]), owned)
  assert.equal(String(life), 'FixLifecycle(owned by its snapshot stream)')
  const [snapshot, ...rest] = [...stream]
  assert.deepEqual(rest, [])
  assert.ok(snapshot.updatedat().equals(clock(10)))
  assert.ok(snapshot.createdat().equals(clock(1)), 'the live chain crossed into the stream')
})

test('a fresh replay is exact and a preprocessed snapshot replay emits nothing', () => {
  const registry = new fix.FixRegistry()
  const raw = [1, 7, 11, 21].map((time) => event(registry, time, 'A'))
  const fill = (life, messages) => messages.map((message) => life.fill(message))
  const same = (left, right) => {
    assert.equal(left.length, right.length)
    for (const [at, message] of left.entries()) assert.ok(message.equals(right[at]), `message ${at}`)
  }
  const full = lifecycle(registry)
  const filled = fill(full, raw)
  assert.equal(filled.length, raw.length)
  assert.ok(filled.every((message) => message.createdat().equals(clock(1))))
  same(fill(lifecycle(registry), raw), filled)
  const fresh = lifecycle(registry)
  same(fill(fresh, filled), filled)
  assert.equal(fresh.alive, full.alive)
  full.clear()
  same(fill(full, raw), filled)
  full.clear()
  same(fill(full, filled), filled)
  const snapshots = () => [...lifecycle(registry).snapshots(raw)]
  const first = snapshots()
  assert.equal(first.length, 3)
  same(snapshots(), first)
  const filtered = lifecycle(registry)
  same(raw.map((message) => filtered.snapshot(message)).filter((held) => held !== null), first)
  filtered.clear()
  same(raw.map((message) => filtered.snapshot(message)).filter((held) => held !== null), first)
  filtered.clear()
  for (const message of filled) assert.equal(filtered.snapshot(message), null)
  assert.equal(filtered.alive, full.alive)
  const next = event(registry, 31, 'A')
  assert.ok(filtered.fill(next).equals(full.fill(next)))
  assert.equal([...lifecycle(registry).snapshots(filled)].length, 0)

  // The codec's lifecycle stream is one lifecycle at the default cadence.
  const codec = new fix.FixCodec(registry)
  const [streamed] = codec.lifecycle([raw[3]])
  assert.ok(streamed.equals(new fix.FixLifecycle(registry).fill(raw[3])))
})

test('the snapshot stream crosses Arrow both ways without a second transition', () => {
  const registry = new fix.FixRegistry()
  const raw = [1, 7, 11].map((time) => event(registry, time, 'A'))
  const schema = raw[0].field
  const codec = new fix.FixCodec(registry)
  const input = codec.arrowReader(schema, raw)
  const snapshots = lifecycle(registry).snapshots(codec.messages(input))
  const reader = codec.arrowReader(schema, snapshots)
  assert.ok(reader instanceof BatchReader)
  const actual = [...new fix.FixCodec(registry).messages(reader)]
  assert.equal(actual.length, 2)
  assert.ok(actual[0].updatedat().equals(clock(0)))
  assert.ok(actual[1].updatedat().equals(clock(10)))
  assert.ok(actual.every((message) => message.createdat().equals(clock(1))))
  for (const message of actual) {
    const row = message.intoRow(message.field)
    const replayed = fix.FixMsg.fromRow(message.field, row, registry)
    assert.ok(replayed.equals(message))
    assert.ok(replayed.intoRow(message.field).equals(row))
  }
  // The same projection of the snapshots taken without the first crossing.
  const expected = [...codec.messages(codec.arrowReader(schema, lifecycle(registry).snapshots(raw)))]
  assert.equal(expected.length, actual.length)
  for (const [at, message] of actual.entries()) assert.ok(message.equals(expected[at]), `message ${at}`)
})

test('normalization keeps the wire, the arrival record, the digest and the real clock', () => {
  const registry = new fix.FixRegistry()
  const codec = new fix.FixCodec(registry)
  const wire = '8=FIX.4.4|35=D|52=20260102-10:15:30.125|60=20260102-10:15:30.123456789|10=0|'
  const raw = withValue(codec.parseFixLine(Buffer.from(wire)), CODE, 'A')
  const filled = new fix.FixLifecycle(registry).fill(raw)
  assert.deepEqual(filled.intoBytes(124), raw.intoBytes(124))
  assert.deepEqual(filled.arrivals(), raw.arrivals())
  assert.deepEqual(filled.digest(), raw.digest())
  assert.ok(filled.byTag(SNAPSHOTAT).equals(raw.byTag(SNAPSHOTAT)))
  assert.ok(filled.createdat().equals(raw.createdat()))
  assert.equal(filled.updatedat().equals(raw.updatedat()), false)
})
