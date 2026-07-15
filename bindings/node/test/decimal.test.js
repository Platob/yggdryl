'use strict'

// Tests for the `yggdryl.decimal` fixed-width scaled decimals (D32/D64/D128/D256), mirroring the
// Rust `io::fixed::decimal` value-type suite and the Python `test_decimal.py` method-for-method:
// construction, checked arithmetic, true numeric ordering, value identity (2.5 === 2.50), the byte
// codec, and cross-width casts.

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { D32, D64, D128, D256 } = yggdryl.decimal
const { DataType } = yggdryl.types

const ALL = [D32, D64, D128, D256]
const BITS = new Map([[D32, 32], [D64, 64], [D128, 128], [D256, 256]])
const MAX_PRECISION = new Map([[D32, 9], [D64, 18], [D128, 38], [D256, 76]])

test('the decimal namespace exposes the four widths', () => {
  for (const cls of ALL) assert.equal(typeof cls, 'function')
})

test('construct: coefficient, scale, precision, bits', () => {
  for (const cls of ALL) {
    const d = new cls(12345n, 2) // 123.45
    assert.equal(d.coefficient, 12345n)
    assert.equal(d.scale, 2)
    assert.equal(d.precision, 5)
    assert.equal(d.bits, BITS.get(cls))
    assert.equal(d.maxPrecision, MAX_PRECISION.get(cls))
    assert.equal(d.toString(), '123.45')
    assert.ok(Math.abs(d.toFloat() - 123.45) < 1e-9)
    assert.equal(new cls(7n).scale, 0) // scale defaults to 0
  }
})

test('fromString and fromFloat', () => {
  for (const cls of ALL) {
    assert.ok(cls.fromString('-0.005').equals(new cls(-5n, 3)))
    assert.equal(cls.fromString('123.45').toString(), '123.45')
    assert.ok(cls.fromFloat(1.5, 1).equals(new cls(15n, 1)))
  }
  assert.throws(() => D128.fromString('1.2.3'), /invalid/)
  assert.throws(() => D128.fromFloat(NaN, 2), /non-finite/)
})

test('arithmetic aligns scales', () => {
  const a = new D128(12345n, 2) // 123.45
  const b = new D128(617n, 2) //     6.17
  assert.equal(a.add(b).toString(), '129.62')
  assert.equal(a.sub(b).toString(), '117.28')
  assert.equal(new D64(25n, 1).add(new D64(25n, 2)).toString(), '2.75') // mixed scales
  assert.equal(new D64(25n, 1).mul(new D64(20n, 1)).toString(), '5.00') // scales add
  assert.equal(a.neg().toString(), '-123.45')
  assert.equal(new D128(-5n, 1).abs().toString(), '0.5')
  assert.equal(new D64(75n, 1).rem(new D64(20n, 1)).toString(), '1.5')
  assert.equal(new D128(1n, 0).div(new D128(3n, 0), 4).toString(), '0.3333')
})

test('checked overflow throws a guided error', () => {
  assert.throws(() => new D128(2n ** 126n, 0).add(new D128(2n ** 126n, 0)), /overflow/)
  assert.throws(() => new D32(3000000000n, 0), /wider decimal/)
})

test('identity is by value across scales', () => {
  const a = new D128(25n, 1) // 2.5
  const b = new D128(250n, 2) // 2.50
  assert.ok(a.equals(b))
  assert.equal(a.hashCode(), b.hashCode())
  assert.deepEqual([...a.serializeBytes()], [...b.serializeBytes()])
  // Ordering is true numeric order.
  assert.equal(new D64(25n, 1).compareTo(new D64(275n, 2)), -1)
  assert.equal(new D64(275n, 2).compareTo(new D64(25n, 1)), 1)
  assert.equal(new D64(25n, 1).compareTo(new D64(2500n, 3)), 0)
})

test('conversions and rescale', () => {
  assert.equal(new D128(12300n, 2).toInt(), 123n) // 123.00 is integral
  assert.throws(() => new D128(12345n, 2).toInt(), /not an exact integer/)
  assert.equal(new D64(12345n, 2).rescale(4).toString(), '123.4500')
  assert.throws(() => new D64(12345n, 2).rescale(1), /drop non-zero/)
  assert.equal(new D64(12345n, 2).roundToScale(1).toString(), '123.5')
  assert.equal(new D64(12345n, 2).truncToScale(1).toString(), '123.4')
  assert.equal(new D64(12345n, 2).trunc().toString(), '123')
  assert.ok(new D64(250n, 2).normalized().equals(new D64(25n, 1)))
})

test('cast between widths', () => {
  const wide = new D32(12345n, 2).toD128()
  assert.equal(wide.toString(), '123.45')
  assert.throws(() => new D128(2n ** 100n, 0).toD32(), /does not fit/)
  // A d256 coefficient beyond i128 casts to itself.
  const big = 10n ** 60n
  assert.equal(new D256(big, 5).coefficient, big)
  assert.equal(new D256(big, 5).toD256().coefficient, big)
})

test('byte codec round-trips for every width', () => {
  for (const cls of ALL) {
    const original = new cls(-123456789n, 4)
    const restored = cls.deserializeBytes(original.serializeBytes())
    assert.ok(restored.equals(original))
  }
})

test('copy is a snapshot', () => {
  const d = new D128(12345n, 2)
  assert.ok(d.copy().equals(d))
})

test('predicates', () => {
  assert.ok(new D128(0n, 0).isZero())
  assert.ok(new D128(-1n, 0).isNegative())
  assert.ok(new D128(1n, 2).isPositive())
})

test('native coercion: toBigInt (truncate) and scientific fromString', () => {
  assert.equal(new D128(19n, 1).toBigInt(), 1n) // truncate 1.9 -> 1
  assert.equal(new D128(-19n, 1).toBigInt(), -1n)
  assert.equal(new D256(10n ** 40n, 0).toBigInt(), 10n ** 40n) // wide, via digits
  assert.equal(new D128(12345n, 2).toFloat(), 123.45)
  // Scientific notation parses (as a stringified big decimal would emit).
  assert.equal(D128.fromString('1.5E+3').toString(), '1500')
  assert.equal(D128.fromString('1.5e-2').toString(), '0.015')
})

test('DataType knows decimals', () => {
  for (const [name, width] of [['d32', 4], ['d64', 8], ['d128', 16], ['d256', 32]]) {
    const dt = DataType.byName(name)
    assert.deepEqual([dt.name, dt.byteWidth, dt.category], [name, width, 'decimal'])
    assert.ok(dt.isDecimal() && dt.isNumeric() && dt.isSigned())
    assert.ok(!dt.isInteger() && !dt.isFloating())
  }
  assert.equal(DataType.d128().name, 'd128')
  assert.ok(DataType.d128().field('amount').isDecimal())
})
