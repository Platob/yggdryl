'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { Decimal32, Decimal64, Decimal128, Decimal256 } = yggdryl.decimal

// Mantissa marshalling: Decimal32 uses `number`, the wider widths use `bigint`.
const NUMERIC = [Decimal64, Decimal128, Decimal256]
const BITS = new Map([
  [Decimal32, 32],
  [Decimal64, 64],
  [Decimal128, 128],
  [Decimal256, 256],
])

test('every width is present under yggdryl.decimal', () => {
  for (const cls of [Decimal32, Decimal64, Decimal128, Decimal256]) {
    assert.equal(typeof cls, 'function')
  }
})

test('construct, value, scale, bits', () => {
  // Decimal32 mantissa is a number.
  const d32 = new Decimal32(12345, 2) // 123.45
  assert.equal(d32.mantissa, 12345)
  assert.equal(d32.scale, 2)
  assert.equal(d32.bits, 32)
  assert.ok(Math.abs(d32.toF64() - 123.45) < 1e-9)
  assert.equal(d32.toI128(), 123n) // toI128 is a bigint

  // Wider widths take a bigint mantissa.
  for (const cls of NUMERIC) {
    const d = new cls(12345n, 2)
    assert.equal(d.mantissa, 12345n)
    assert.equal(d.scale, 2)
    assert.equal(d.bits, BITS.get(cls))
    assert.ok(Math.abs(d.toF64() - 123.45) < 1e-9)
    assert.equal(new cls(7n).scale, 0) // scale defaults to 0
  }
  assert.equal(new Decimal32(7).scale, 0)
})

test('fromF64', () => {
  assert.ok(Decimal32.fromF64(1.5, 1).equals(new Decimal32(15, 1)))
  assert.ok(Decimal64.fromF64(1.5, 1).equals(new Decimal64(15n, 1)))
})

test('rescale and overflow', () => {
  const d = new Decimal64(123n, 0)
  assert.ok(d.rescale(2).equals(new Decimal64(12300n, 2)))
  assert.ok(d.rescale(2).rescale(0).equals(d))
  assert.throws(() => new Decimal32(2000000000, 0).rescale(2), /wider decimal/)
})

test('constructor range check is guided', () => {
  // Overflow routes through the core, so the message matches Python's ("wider decimal").
  assert.throws(() => new Decimal32(2 ** 40, 0), /wider decimal/)
  assert.throws(() => new Decimal32(3.5, 0), /whole number/) // fractional
  assert.throws(() => new Decimal64(2n ** 80n, 0), /wider decimal/)
  assert.throws(() => new Decimal128(2n ** 200n, 0), /wider decimal/) // >128 bits
})

test('byte round trip and length', () => {
  const cases = [
    [new Decimal32(-4200, 2), 5],
    [new Decimal64(-4200n, 2), 9],
    [new Decimal128(-4200n, 2), 17],
    [new Decimal256(-4200n, 2), 33],
  ]
  for (const [d, n] of cases) {
    const raw = d.serializeBytes()
    assert.equal(raw.length, n)
    assert.ok(d.constructor.deserializeBytes(raw).equals(d))
  }
  assert.throws(() => Decimal32.deserializeBytes(Buffer.from([0, 0, 0])), /expected 5/)
})

test('value semantics', () => {
  const a = new Decimal64(12345n, 2)
  assert.ok(a.equals(new Decimal64(12345n, 2)))
  assert.ok(!a.equals(new Decimal64(12345n, 3))) // scale matters (rule 7)
  assert.equal(a.hashCode(), new Decimal64(12345n, 2).hashCode())
  assert.equal(new Decimal32(12345, 2).toString(), '123.45')
  assert.equal(new Decimal64(-5n, 2).toString(), '-0.05')
})

test('cross-width widen and narrow', () => {
  assert.ok(new Decimal32(12345, 2).toDecimal256().equals(new Decimal256(12345n, 2)))
  assert.ok(new Decimal64(999n, 1).toDecimal256().equals(new Decimal256(999n, 1)))
  assert.ok(new Decimal128(999n, 1).toDecimal256().equals(new Decimal256(999n, 1)))

  assert.ok(new Decimal256(999n, 1).tryToDecimal128().equals(new Decimal128(999n, 1)))
  const huge = new Decimal256(2n * 2n ** 126n, 0) // > i128 max
  assert.throws(() => huge.tryToDecimal128(), /wider decimal/)
})

test('decimal256 beyond i128 round-trips via the bigint bridge', () => {
  const mantissa = 2n ** 200n + 123n
  const d = new Decimal256(mantissa, 3)
  assert.equal(d.mantissa, mantissa) // exact
  assert.equal(d.toI128(), null) // integer part exceeds i128
  assert.ok(Decimal256.deserializeBytes(d.serializeBytes()).equals(d))

  // Negative 256-bit magnitudes survive (two's-complement <-> sign-magnitude).
  const neg = new Decimal256(-(2n ** 200n), 0)
  assert.equal(neg.mantissa, -(2n ** 200n))
  assert.ok(Decimal256.deserializeBytes(neg.serializeBytes()).equals(neg))

  // i256::MIN edge: -2^255 round-trips exactly.
  const min = new Decimal256(-(2n ** 255n), 0)
  assert.equal(min.mantissa, -(2n ** 255n))
  assert.ok(Decimal256.deserializeBytes(min.serializeBytes()).equals(min))
})

test('decimal256 out-of-range mantissa is guided', () => {
  assert.throws(() => new Decimal256(2n ** 256n, 0), /out of range for decimal256/)
})
