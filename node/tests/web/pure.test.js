// `node/web/{diff,candles,scenario,shortcuts,scales}.js`: the pure modules the components use.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { aggregateCandles, bucketOf } from '../../web/candles.js'
import { sumDecimal } from '../../web/decimal.js'
import { diffBooks, diffStreams } from '../../web/diff.js'
import { linear, niceStep, padded, ticks } from '../../web/scales.js'
import {
  createScenario,
  earliestAffected,
  insertEvent,
  parseScenario,
  removeEvent,
  serializeScenario,
} from '../../web/scenario.js'
import { SHORTCUTS, chordOf, commandFor, displayChord } from '../../web/shortcuts.js'

const T0 = 1_700_000_000_000_000_000n

function book(hash, bids, asks, extra = {}) {
  return {
    currunix: T0.toString(),
    curruuid: 'b-1',
    ticker: 'ALPHA',
    stableHash: hash,
    spread: '2',
    bid: {
      limits: bids.map(([price, quantity, ...uuids]) => ({ price, quantity, uuids })),
      live: bids.flatMap(([price, quantity, ...uuids]) => uuids.map((curruuid) => ({ curruuid, price, quantity }))),
      deltas: [],
    },
    ask: {
      limits: asks.map(([price, quantity, ...uuids]) => ({ price, quantity, uuids })),
      live: asks.flatMap(([price, quantity, ...uuids]) => uuids.map((curruuid) => ({ curruuid, price, quantity }))),
      deltas: [],
    },
    executions: [],
    ...extra,
  }
}

test('two books diff by native identity and exact texts, never rendered text', () => {
  const base = book('1', [['100', '8', 'o-1']], [['102', '4', 'q-1']])
  const same = book('1', [['100', '8', 'o-1']], [['102', '4', 'q-1']])
  assert.equal(diffBooks(base, same).same, true)
  // A stated hash that differs is a difference even when every fact reads alike.
  assert.equal(diffBooks(base, book('2', [['100', '8', 'o-1']], [['102', '4', 'q-1']])).same, false)
  const other = book('3', [['100', '10', 'o-1', 'o-2'], [null, '5', 'm-1']], [], { spread: null })
  const diff = diffBooks(base, other)
  assert.equal(diff.same, false)
  assert.deepEqual(
    diff.facts.map((change) => change.name),
    ['spread', 'stableHash'],
  )
  assert.deepEqual(diff.bid.limits.changed.map((change) => change.key), ['100'])
  assert.deepEqual(diff.bid.limits.added.map((limit) => limit.key), [null])
  assert.deepEqual(diff.bid.live.added.map((entry) => entry.curruuid), ['o-2', 'm-1'])
  assert.deepEqual(diff.ask.limits.removed.map((limit) => limit.key), ['102'])
  assert.deepEqual(diff.ask.live.removed.map((entry) => entry.curruuid), ['q-1'])
  // `100` and `100.0` are two texts: the browser does not decide they are one number.
  assert.equal(diffBooks(base, book('1', [['100.0', '8', 'o-1']], [['102', '4', 'q-1']])).same, false)
})

test('streams align by instant and report each side of a missing book', () => {
  const a = book('1', [], [])
  const b = { ...book('2', [], []), currunix: (T0 + 1n).toString() }
  const rows = diffStreams([a, b], [a])
  assert.deepEqual(
    rows.map((row) => [row.at, row.base, row.scenario, row.changes.same]),
    [
      [T0.toString(), '1', '1', true],
      [(T0 + 1n).toString(), '2', null, false],
    ],
  )
})

test('candles bucket by bigint instant and compare prices exactly', () => {
  const executions = [
    { currunix: (T0 + 1n).toString(), lastpx: '100.5', lastqty: '2' },
    { currunix: (T0 + 2n).toString(), lastpx: '100.25', lastqty: '3' },
    { currunix: (T0 + 3n).toString(), lastpx: '100.75', lastqty: null },
    { currunix: (T0 + 1_000_000_005n).toString(), lastpx: null, lastqty: '9' },
    { currunix: (T0 + 1_000_000_006n).toString(), lastpx: '99', lastqty: '1.5' },
  ]
  const candles = aggregateCandles(executions, { origin: T0, bucketNs: 1_000_000_000n })
  assert.equal(candles.length, 2)
  const [first, second] = candles
  assert.equal(first.open, T0)
  assert.equal(first.close, T0 + 1_000_000_000n)
  assert.deepEqual([first.first, first.last, first.high, first.low, first.count], ['100.5', '100.75', '100.75', '100.25', 3])
  assert.deepEqual([second.first, second.count], ['99', 1])
  // No volume: nothing draws one, and the depth chart's step is the one sum the browser makes.
  assert.equal('volume' in first, false)
  assert.equal(bucketOf(T0 - 1n, T0, 10n), -1n)
  assert.equal(sumDecimal(['0.1', '0.2']), '0.3', 'no float adds them')
  assert.equal(sumDecimal(['9007199254740993', '1']), '9007199254740994')
  assert.equal(sumDecimal(['-1.5', '1.25']), '-0.25')
})

test('a candle is the last executed price alone: an order price never fills it', () => {
  // `lastpx` and `lastqty` are what the execution traded; an execution stating none trades nothing here.
  const executions = [
    { currunix: (T0 + 1n).toString(), price: '100', quantity: '5', lastpx: null, lastqty: null },
    { currunix: (T0 + 2n).toString(), price: '101', quantity: '1' },
  ]
  assert.deepEqual(aggregateCandles(executions, { origin: T0, bucketNs: 1_000_000_000n }), [])
  const traded = aggregateCandles([...executions, { currunix: (T0 + 3n).toString(), price: '90', lastpx: '99.5', lastqty: null }], {
    origin: T0,
    bucketNs: 1_000_000_000n,
  })
  assert.deepEqual(traded.map((candle) => [candle.first, candle.high, candle.low, candle.last, candle.count]), [['99.5', '99.5', '99.5', '99.5', 1]])
})

test('a scenario keeps its events by instant and reloads exactly', () => {
  let scenario = createScenario(' spike ')
  assert.equal(scenario.name, 'spike')
  const at = (n) => ({ curruuid: `e-${n}`, currunix: (T0 + BigInt(n)).toString(), native: 'AAAA', kind: 'order_event', facts: { side: 'BUY' } })
  scenario = insertEvent(scenario, at(5))
  scenario = insertEvent(scenario, at(1))
  scenario = insertEvent(scenario, { ...at(5), curruuid: 'e-5b' })
  assert.deepEqual(scenario.events.map((event) => event.curruuid), ['e-1', 'e-5', 'e-5b'], 'stable for ties')
  assert.throws(() => insertEvent(scenario, at(5)), /already in scenario/)
  assert.throws(() => insertEvent(scenario, { ...at(7), native: '' }), /native leaf/)
  const reloaded = parseScenario(serializeScenario(scenario))
  assert.deepEqual(reloaded, scenario)
  const removed = removeEvent(scenario, 'e-1')
  assert.deepEqual(removed.events.map((event) => event.curruuid), ['e-5', 'e-5b'])
  assert.equal(removeEvent(scenario, 'none'), scenario)
  assert.equal(earliestAffected(scenario, removed), T0 + 1n)
  assert.equal(earliestAffected(removed, scenario), T0 + 1n)
  assert.equal(earliestAffected(scenario, scenario), undefined)
  assert.throws(() => parseScenario('{"name":1}'), /expected a scenario/)
})

test('a scenario re-runs from the instant the walk reads an event at: its grid snapunix before its currunix', () => {
  const gridded = { curruuid: 'e-g', currunix: (T0 + 10n).toString(), snapunix: (T0 + 5n).toString(), native: 'AAAA', kind: 'order_event', facts: {} }
  const base = createScenario('grid')
  const inserted = insertEvent(base, gridded)
  assert.equal(earliestAffected(base, inserted), T0 + 5n, 'inserted')
  assert.equal(earliestAffected(inserted, base), T0 + 5n, 'removed')
  // Moved on the grid alone: both of its instants, the earlier one first.
  const moved = { ...inserted, events: [{ ...gridded, snapunix: (T0 + 7n).toString() }] }
  assert.equal(earliestAffected(inserted, moved), T0 + 5n)
  assert.equal(earliestAffected(moved, inserted), T0 + 5n)
  assert.equal(earliestAffected(inserted, { ...inserted, events: [{ ...gridded }] }), undefined, 'unchanged')
})

test('shortcuts map chords to commands and stay out of text fields', () => {
  const key = (props) => ({ key: ' ', ctrlKey: false, metaKey: false, shiftKey: false, altKey: false, target: null, ...props })
  assert.equal(chordOf(key({ key: ' ' })), 'Space')
  assert.equal(chordOf(key({ key: 'k', ctrlKey: true }), false), 'Mod+k')
  assert.equal(chordOf(key({ key: 'k', metaKey: true }), true), 'Mod+k')
  assert.equal(chordOf(key({ key: 'ArrowRight', shiftKey: true })), 'Shift+ArrowRight')
  assert.equal(chordOf(key({ key: '?', shiftKey: true })), '?')
  assert.equal(commandFor(key({ key: ' ' })), 'transport.toggle')
  assert.equal(commandFor(key({ key: 'K', ctrlKey: true, shiftKey: false }), false), 'palette.open')
  const field = { tagName: 'INPUT', isContentEditable: false, closest: () => null }
  assert.equal(commandFor(key({ key: 'g', target: field })), undefined)
  assert.equal(commandFor(key({ key: 'Escape', target: field })), 'ui.close')
  // Space and Enter belong to a control that acts on them itself: a button, a link, a summary, anything with a role.
  const inside = (selector) => ({ tagName: 'SPAN', isContentEditable: false, closest: (wanted) => (wanted.includes(selector) ? {} : null) })
  for (const selector of ['button', 'a[href]', 'summary', '[role]']) {
    assert.equal(commandFor(key({ key: ' ', target: inside(selector) })), undefined, `Space inside ${selector}`)
    assert.equal(commandFor(key({ key: 'Enter', target: inside(selector) })), undefined, `Enter inside ${selector}`)
    assert.equal(commandFor(key({ key: 'g', target: inside(selector) })), 'transport.grid', `a letter inside ${selector} is still a shortcut`)
  }
  const page = { tagName: 'BODY', isContentEditable: false, closest: () => null }
  assert.equal(commandFor(key({ key: ' ', target: page })), 'transport.toggle', 'Space on the page plays')
  assert.equal(displayChord('Mod+k', false), 'Ctrl+k')
  assert.equal(displayChord('Mod+k', true), '⌘k')
  assert.equal(new Set(SHORTCUTS.map(([chord]) => chord)).size, SHORTCUTS.length, 'one command per chord')
})

test('scales are linear, invertible and tick at round values', () => {
  const y = linear(100, 200, 300, 0)
  assert.equal(y(150), 150)
  assert.equal(y.invert(0), 200)
  assert.deepEqual(ticks(0, 10, 5), [0, 2, 4, 6, 8, 10])
  assert.deepEqual(ticks(99.5, 100.5, 4), [99.6, 99.8, 100, 100.2, 100.4])
  assert.equal(niceStep(0.03), 0.02)
  assert.deepEqual(padded(100, 100), [95, 105])
  assert.deepEqual(padded(100, 200, 0.1), [90, 210])
  assert.ok(Number.isNaN(y(NaN)))
  assert.equal(linear(0, 0, 0, 10)(0), 5, 'a flat domain lands mid-range')
})

test('snapshot partitions diff by symbol and scope, as the served row names them', () => {
  const partition = { symbol: 'ALPHA', scope: 'Symbol=ALPHA' }
  const base = book('1', [], [], { snapshotpartitions: [partition] })
  const other = book('2', [], [], { snapshotpartitions: [{ symbol: null, scope: 'Symbol=ALPHA' }] })
  const diff = diffBooks(base, other)
  assert.deepEqual(diff.facts.map((change) => change.name), ['stableHash'], 'never one flat fact')
  assert.deepEqual(diff.snapshotpartitions.removed.map((held) => [held.symbol, held.scope]), [['ALPHA', 'Symbol=ALPHA']])
  assert.deepEqual(diff.snapshotpartitions.added.map((held) => [held.symbol, held.scope]), [[null, 'Symbol=ALPHA']])
  assert.equal(diffBooks(base, book('1', [], [], { snapshotpartitions: [{ ...partition }] })).same, true)
  // Two books that differ only there are not the same even under one stated hash.
  assert.equal(diffBooks(base, book('1', [], [], { snapshotpartitions: [] })).same, false)
})
