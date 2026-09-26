// `node/web/instant.js`: bigint instants, differences to pixels, formatting.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  compareInstant,
  elapsed,
  formatClock,
  formatInstant,
  fromPixels,
  instantText,
  nanosPerPixel,
  parseInstant,
  toPixels,
} from '../../web/instant.js'

const T0 = 1_700_000_000_000_000_000n

test('two instants one nanosecond apart stay distinct and ordered', () => {
  const a = parseInstant('1700000000000000000')
  const b = parseInstant('1700000000000000001')
  assert.notEqual(a, b)
  assert.equal(compareInstant(a, b), -1)
  assert.equal(compareInstant(b, a), 1)
  assert.equal(compareInstant(a, '1700000000000000000'), 0)
  assert.equal(instantText(b), '1700000000000000001')
  assert.equal(elapsed(a, b), 1n)
  // A Number would fold them together; the text keeps every digit.
  assert.equal(Number(a), Number(b))
  assert.notEqual(formatInstant(a), formatInstant(b))
})

test('formatting keeps all nine fractional digits and negative instants floor', () => {
  assert.equal(formatInstant(T0), '2023-11-14T22:13:20.000000000Z')
  assert.equal(formatInstant(T0 + 123_456_789n), '2023-11-14T22:13:20.123456789Z')
  assert.equal(formatClock(T0 + 123_456_789n), '22:13:20.123')
  assert.equal(formatInstant(-1n), '1969-12-31T23:59:59.999999999Z')
})

test('pixels scale a difference, never the instant, to a thousandth', () => {
  const perPixel = 1_000_000n // one millisecond per pixel
  assert.equal(toPixels(T0, T0, perPixel), 0)
  assert.equal(toPixels(T0 + 5_000_000n, T0, perPixel), 5)
  assert.equal(toPixels(T0 + 1_500_000n, T0, perPixel), 1.5)
  assert.equal(toPixels(T0 - 2_000_000n, T0, perPixel), -2)
  assert.equal(fromPixels(5, T0, perPixel), T0 + 5_000_000n)
  assert.equal(fromPixels(1.5, T0, perPixel), T0 + 1_500_000n)
  assert.equal(nanosPerPixel(T0, T0 + 1_000_000_000n, 1000), 1_000_000n)
  assert.equal(nanosPerPixel(T0, T0, 1000), 1n, 'an empty window still maps a pixel')
  assert.throws(() => toPixels(T0, T0, 0n), /positive bigint/)
})

test('refusals name what was expected', () => {
  assert.throws(() => parseInstant('17e5'), /decimal nanoseconds/)
  assert.throws(() => parseInstant(12), /decimal nanoseconds/)
  assert.throws(() => instantText('1'), /bigint instant/)
})
