// Tests for the yggdryl Scalar (a single atomic value). Build first with `npm run
// build`, then `node --test`.

const { test } = require('node:test')
const assert = require('node:assert')
const { Scalar, DataType } = require('..')

test('infers and reads primitives', () => {
  assert.strictEqual(new Scalar(42).dataType.toString(), 'int64')
  assert.strictEqual(new Scalar(42).value, 42)
  assert.strictEqual(new Scalar(1.5).dataType.toString(), 'float64')
  assert.strictEqual(new Scalar(true).value, true)
  assert.strictEqual(new Scalar('hi').value, 'hi')
})

test('explicit dtype builds a specific type', () => {
  assert.strictEqual(new Scalar(5, 'int32').dataType.toString(), 'int32')
  assert.strictEqual(new Scalar(5, DataType.int(32, true)).dataType.toString(), 'int32')
  assert.strictEqual(new Scalar(1.5, 'float32').dataType.toString(), 'float32')
})

test('typed null and accessors', () => {
  const n = Scalar.null('int64')
  assert.ok(n.isNull)
  assert.strictEqual(n.value, null)
  assert.strictEqual(n.dataType.toString(), 'int64')

  const i = new Scalar(7, 'int32')
  assert.strictEqual(i.asInt(), 7)
  assert.strictEqual(i.asFloat(), null)
  assert.strictEqual(new Scalar('x').asStr(), 'x')
  assert.strictEqual(new Scalar(true).asBool(), true)
})

test('canonical string round-trip', () => {
  const s = new Scalar(42)
  assert.strictEqual(s.toStr(), '42::int64')
  assert.ok(Scalar.fromStr('42::int64').equals(s))
})

test('bytes / toJSON round-trip including a temporal from_str', () => {
  const ts = Scalar.fromStr('1700000000::timestamp[s]')
  assert.ok(Scalar.fromBytes(ts.toBytes()).equals(ts))
  assert.ok(Scalar.fromJSON(ts.toJSON()).equals(ts))
})

test('binary factory and byte access', () => {
  const b = Scalar.binary(Buffer.from('xy'))
  assert.strictEqual(b.dataType.toString(), 'binary')
  assert.deepStrictEqual([...b.asBytes()], [120, 121])
})

test('decimal value renders scaled', () => {
  const d = Scalar.fromStr('12345::decimal128[7, 2]')
  assert.strictEqual(d.value, '123.45')
})

test('component map round-trip', () => {
  const s = new Scalar(99, 'int32')
  const m = s.toMapping()
  assert.strictEqual(m.type, 'int32')
  assert.ok(Scalar.fromMapping(m).equals(s))
})

test('scalar arithmetic', () => {
  const a = new Scalar(6)
  const b = new Scalar(4)
  assert.strictEqual(a.add(b).value, 10)
  assert.strictEqual(a.sub(b).value, 2)
  assert.strictEqual(a.mul(b).value, 24)
  assert.strictEqual(a.div(b).value, 1)
  assert.strictEqual(a.neg().value, -6)
  // mixed int + float promotes to float
  const mixed = a.add(new Scalar(1.5))
  assert.strictEqual(mixed.dataType.toString(), 'float64')
  assert.strictEqual(mixed.value, 7.5)
  // division by zero and an undefined combination throw
  assert.throws(() => a.div(new Scalar(0)))
  assert.throws(() => new Scalar('x').add(a))
})
