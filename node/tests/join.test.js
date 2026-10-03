'use strict'

// Pins node/src/join.rs: the kind a join keeps rows by, read through the
// core's own words, and the options beside it, one plain object read once
// into the core's `JoinOptions` - the helpers every `joinWith` shares. The
// keys cross as one Scalar the core reads by its own rule.

const assert = require('node:assert/strict')
const test = require('node:test')

const binding = require('yggdryl')
const { ChunkedSerie, Field, Serie, SerieReader, SpillOptions } = binding

const trades = () =>
  Serie.fromScalars(Field.from('trade: struct<id: int64 not null, venue: utf8 not null> not null'), [
    { id: 1, venue: 'XNAS' },
    { id: 2, venue: 'XNYS' },
    { id: 3, venue: 'XNAS' },
  ])
const venues = () =>
  Serie.fromScalars(
    Field.from('venue: struct<venue: utf8 not null, city: utf8 not null> not null'),
    [
      { venue: 'XNAS', city: 'New York' },
      { venue: 'XPAR', city: 'Paris' },
    ],
  )
const markets = () =>
  Serie.fromScalars(
    Field.from('market: struct<market: utf8 not null, city: utf8 not null> not null'),
    [{ market: 'XNAS', city: 'New York' }],
  )

// Rows in one canonical order - each row's cells by name - so a build side
// read in another order answers the same set.
const canonical = (row) => JSON.stringify(Object.keys(row).sort().map((name) => [name, row[name]]))
const sortedRows = (rows) =>
  [...rows].sort((left, right) => canonical(left).localeCompare(canonical(right)))
const rowsOf = (serie) => sortedRows(serie.asJs())

test('every word of the six kinds reads as the core reads it', () => {
  const inner = [
    { id: 1, venue: 'XNAS', city: 'New York' },
    { id: 3, venue: 'XNAS', city: 'New York' },
  ]
  for (const how of [undefined, null, 'inner', 'Inner Join']) {
    assert.deepEqual(trades().joinWith(venues(), 'venue', how).asJs(), inner, String(how))
  }
  assert.deepEqual(trades().joinWith(venues(), 'venue', 'left outer join').asJs(), [
    { id: 1, venue: 'XNAS', city: 'New York' },
    { id: 2, venue: 'XNYS', city: null },
    { id: 3, venue: 'XNAS', city: 'New York' },
  ])
  // An unmatched right row comes after the probe rows, its left columns
  // null - the shared key coalesced from the right.
  for (const how of ['full', 'outer', 'FULL OUTER']) {
    assert.deepEqual(
      rowsOf(trades().joinWith(venues(), 'venue', how)),
      sortedRows([
        { id: 1, venue: 'XNAS', city: 'New York' },
        { id: 2, venue: 'XNYS', city: null },
        { id: 3, venue: 'XNAS', city: 'New York' },
        { id: null, venue: 'XPAR', city: 'Paris' },
      ]),
    )
  }
  assert.deepEqual(
    rowsOf(trades().joinWith(venues(), 'venue', 'right')),
    sortedRows([
      { id: 1, venue: 'XNAS', city: 'New York' },
      { id: 3, venue: 'XNAS', city: 'New York' },
      { id: null, venue: 'XPAR', city: 'Paris' },
    ]),
  )
  assert.deepEqual(trades().joinWith(venues(), 'venue', 'semi').asJs(), [
    { id: 1, venue: 'XNAS' },
    { id: 3, venue: 'XNAS' },
  ])
  assert.deepEqual(trades().joinWith(venues(), 'venue', 'anti').asJs(), [{ id: 2, venue: 'XNYS' }])
  assert.throws(
    () => trades().joinWith(venues(), 'venue', 'cross'),
    /one of `inner`, `left`, `right`, `full` \(or `outer`\), `semi`, `anti`/,
  )
  assert.throws(() => trades().joinWith(venues(), 'venue', true), /how must be a join kind's word/)
})

test('every spelling of the keys is one key list, read by the core', () => {
  const expected = [
    { id: 1, venue: 'XNAS', market: 'XNAS', city: 'New York' },
    { id: 3, venue: 'XNAS', market: 'XNAS', city: 'New York' },
  ]
  for (const by of [
    'venue = market',
    ['venue = market'],
    [['venue', 'market']],
    new Map([['venue', 'market']]),
    { venue: 'market' },
  ]) {
    assert.deepEqual(trades().joinWith(markets(), by).asJs(), expected, String(by))
  }
  // A bare term is the same term over both sides, and coalesces.
  for (const by of ['venue', ['venue'], [['venue', 'venue']]]) {
    assert.deepEqual(trades().joinWith(venues(), by).asJs().length, 2)
  }
  assert.throws(() => trades().joinWith(venues(), []))
  assert.throws(() => trades().joinWith(venues(), 'venue < venue'), /a join key is an equality/)
})

test('the options are read once, each absent or null slot the default', () => {
  const plain = trades().joinWith(venues(), 'venue')
  for (const options of [undefined, null, {}, { coalesce: null, suffix: null, build: null }]) {
    assert.ok(trades().joinWith(venues(), 'venue', 'inner', options).equals(plain))
  }
  // Not coalesced, the right key is a column of its own, suffixed where it
  // collides.
  const apart = trades().joinWith(venues(), 'venue', 'inner', { coalesce: false })
  assert.deepEqual(apart.names, ['id', 'venue', 'venue_right', 'city'])
  const suffixed = trades().joinWith(venues(), 'venue', 'inner', { coalesce: false, suffix: '_r' })
  assert.deepEqual(suffixed.names, ['id', 'venue', 'venue_r', 'city'])
  // The build side, pruning and the pushdown bound change no answer.
  for (const options of [
    { build: 'left' },
    { build: 'right' },
    { prune: false },
    { pushdownKeys: 0 },
  ]) {
    assert.deepEqual(
      rowsOf(trades().joinWith(venues(), 'venue', 'left', options)),
      rowsOf(trades().joinWith(venues(), 'venue', 'left')),
      JSON.stringify(options),
    )
  }
  // The output settles under the join's own bound.
  const spilled = trades().joinWith(venues(), 'venue', 'inner', {
    spill: new SpillOptions({ byteSize: 0 }),
  })
  assert.equal(spilled.isSpilled(), true)
  assert.ok(spilled.equals(plain))
  const resident = trades().joinWith(venues(), 'venue', 'inner', {
    spill: new SpillOptions({ byteSize: SpillOptions.NEVER }),
  })
  assert.equal(resident.isSpilled(), false)
})

test('a refused option names itself, before any row is read', () => {
  const join = (options) => trades().joinWith(venues(), 'venue', 'inner', options)
  assert.throws(() => join({ build: 'middle' }), /`left` or `right`/)
  assert.throws(() => join({ pushdownKeys: -1 }), /pushdownKeys must be a non-negative whole number/)
  assert.throws(() => join({ pushdownKeys: 1.5 }), /pushdownKeys must be a non-negative whole number/)
  assert.throws(() => join({ how: 'left' }), /take coalesce, suffix, build, prune, spill and pushdownKeys, got "how"/)
  assert.throws(() => join('left'), /must be an object of coalesce/)
  assert.throws(() => join({ spill: 0 }))
  // The same reading serves every joinWith.
  assert.throws(
    () => ChunkedSerie.fromSerie(trades()).joinWith(ChunkedSerie.fromSerie(venues()), 'venue', 'inner', { build: 'middle' }),
    /`left` or `right`/,
  )
  const stream = SerieReader.fromSerie(trades())
  assert.throws(() => stream.joinWith(venues(), 'venue', 'inner', { build: 'middle' }), /`left` or `right`/)
  assert.equal([...stream].length, 1)
})
