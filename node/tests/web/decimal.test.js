// `node/web/decimal.js`: decimal text displayed, compared exactly, floated for pixels only.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  compareDecimal,
  decimalParts,
  formatDecimal,
  isDecimalText,
  maxDecimal,
  minDecimal,
  toFloat,
} from '../../web/decimal.js'

test('parts read the sign and the digits, trimming what states nothing', () => {
  assert.deepEqual(decimalParts('82.50'), { negative: false, integer: '82', fraction: '5' })
  assert.deepEqual(decimalParts('-0.000000000000000001'), {
    negative: true,
    integer: '0',
    fraction: '000000000000000001',
  })
  assert.deepEqual(decimalParts('-0.0'), { negative: false, integer: '0', fraction: '' })
  assert.deepEqual(decimalParts('007'), { negative: false, integer: '7', fraction: '' })
  assert.ok(isDecimalText('1.5') && !isDecimalText('1e5') && !isDecimalText('abc'))
  assert.throws(() => decimalParts('1e5'), /decimal text/)
})

test('display keeps every digit and pads to the asked places', () => {
  assert.equal(formatDecimal('82.5'), '82.5')
  assert.equal(formatDecimal('82.5', { places: 2 }), '82.50')
  assert.equal(formatDecimal('82.500000000000000000'), '82.5')
  assert.equal(formatDecimal('0.123456789012345678', { places: 2 }), '0.123456789012345678')
  assert.equal(formatDecimal('1234567.5', { group: true }), '1 234 567.5')
  assert.equal(formatDecimal(null), '')
})

test('comparison is exact where a float would not be', () => {
  assert.equal(compareDecimal('0.1', '0.10'), 0)
  assert.equal(compareDecimal('9007199254740993', '9007199254740992'), 1)
  assert.equal(compareDecimal('-1', '1'), -1)
  assert.equal(compareDecimal('-2.5', '-2.25'), -1)
  assert.equal(compareDecimal('100', '99.999999999999999999'), 1)
  assert.equal(maxDecimal('1.5', '1.25', '1.50'), '1.5')
  assert.equal(minDecimal('1.5', '1.25', '1.50'), '1.25')
  assert.equal(Number('9007199254740993'), Number('9007199254740992'), 'the float folds them')
})

test('a float is read only for a pixel and refuses non-decimal text', () => {
  assert.equal(toFloat('82.5'), 82.5)
  assert.ok(Number.isNaN(toFloat(null)))
  assert.throws(() => toFloat('abc'), /decimal text/)
})
