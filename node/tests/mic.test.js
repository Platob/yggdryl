'use strict'

// `node/src/mic.rs`: one ISO 10383 market identifier code, read as the `mic`
// datatype reads one, redirected to the core.

const assert = require('node:assert/strict')
const test = require('node:test')

const { Country, Mic } = require('yggdryl')

test('a segment answers its operating MIC and its country', () => {
  const segment = new Mic('XNGS')
  assert.equal(segment.toString(), 'XNGS')
  assert.equal(segment.toJSON(), 'XNGS')
  assert.ok(segment.isSegment)
  assert.equal(segment.operating, 'XNAS')
  assert.equal(segment.country, 'US')
  assert.equal(new Country(segment.country).currency, 'USD', 'a market leads to its currency')
  const operating = new Mic('XNAS')
  assert.ok(!operating.isSegment)
  assert.equal(operating.operating, 'XNAS', 'an operating MIC is its own')
  assert.equal(new Mic('XLON').country, 'GB')
  assert.ok(segment.equals(new Mic('XNGS')))
  assert.ok(!segment.equals(operating))
})

test('an off-exchange or unassigned code places no country', () => {
  // ISO 10383 lists XOFF in the registry's own ZZ, which is no country.
  const off = new Mic('XOFF')
  assert.equal(off.operating, 'XOFF')
  assert.ok(!off.isSegment)
  assert.equal(off.country, null)
  const none = new Mic('XXXX')
  assert.ok(none.isNone)
  assert.equal(none.country, null)
  assert.ok(!new Mic('XNAS').isNone)
  // A code the registry never assigned answers none, and is no segment.
  const unknown = new Mic('QQQQ')
  assert.equal(unknown.operating, null)
  assert.ok(!unknown.isSegment)
  assert.equal(unknown.country, null)
  assert.throws(() => new Mic('TOOLONG'))
})
