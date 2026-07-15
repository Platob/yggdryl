'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { DataType, Utf8Scalar, Utf8Serie, BinaryScalar, BinarySerie } = yggdryl.types

// ---------------------------------------------------------------------------------------
// Scalar
// ---------------------------------------------------------------------------------------

test('the types namespace exposes the variable-length value classes', () => {
  for (const cls of [Utf8Scalar, Utf8Serie, BinaryScalar, BinarySerie]) {
    assert.equal(typeof cls, 'function')
  }
})

test('utf8 scalar', () => {
  const s = new Utf8Scalar('héllo')
  assert.ok(s.value === 'héllo' && !s.isNull && s.typeName === 'utf8')
  assert.ok(s.dataType.equals(DataType.utf8()))
  for (const nul of [new Utf8Scalar(), new Utf8Scalar(null), Utf8Scalar.null()]) {
    assert.ok(nul.isNull && nul.value === null)
  }
  assert.ok(s.equals(new Utf8Scalar('héllo')) && s.hashCode() === new Utf8Scalar('héllo').hashCode())
  assert.ok(!s.equals(new Utf8Scalar('other')) && !s.equals(Utf8Scalar.null()))
})

test('binary scalar', () => {
  const raw = Buffer.from([0xff, 0x00, 0x41])
  const b = new BinaryScalar(raw)
  assert.ok(b.value.equals(raw) && b.typeName === 'binary')
  assert.ok(b.dataType.equals(DataType.binary()))
  assert.ok(!b.equals(new BinaryScalar(Buffer.from('other'))))
})

test('scalar byte codec (value and null)', () => {
  for (const [cls, value] of [[Utf8Scalar, 'hi'], [Utf8Scalar, ''], [BinaryScalar, Buffer.from([0, 0xff])]]) {
    const s = new cls(value)
    assert.ok(cls.deserializeBytes(s.serializeBytes()).equals(s))
    assert.ok(cls.deserializeBytes(cls.null().serializeBytes()).equals(cls.null()))
  }
})

test('scalar field, and invalid UTF-8 is rejected', () => {
  const field = new Utf8Scalar('x').field('name', false)
  assert.ok(field.name === 'name' && field.typeName === 'utf8' && field.nullable === false)

  const bad = new BinaryScalar(Buffer.from([0xff, 0xfe])).serializeBytes()
  assert.throws(() => Utf8Scalar.deserializeBytes(bad))
})

// ---------------------------------------------------------------------------------------
// Serie
// ---------------------------------------------------------------------------------------

test('utf8 serie', () => {
  const col = new Utf8Serie(['a', null, 'cd'])
  assert.ok(col.length === 3 && col.nullCount === 1 && col.hasNulls)
  assert.deepEqual(col.toOptions(), ['a', null, 'cd'])
  assert.ok(col.get(0) === 'a' && col.get(1) === null)
  assert.ok(col.getScalar(0).equals(new Utf8Scalar('a')))
  assert.ok(col.getScalar(1).equals(Utf8Scalar.null()))
  assert.ok(new Utf8Serie().isEmpty())
})

test('serie mutation rewrites offsets', () => {
  const col = new Utf8Serie(['a', 'bb', 'ccc'])
  col.set(1, 'longer') // grows
  col.set(2, null) // -> null
  col.push('z')
  assert.deepEqual(col.toOptions(), ['a', 'longer', null, 'z'])
  assert.throws(() => col.set(99, 'x'))
})

test('binary serie', () => {
  const col = new BinarySerie([Buffer.from([1]), null, Buffer.from([0xff, 0xfe])])
  const opts = col.toOptions()
  assert.ok(opts[0].equals(Buffer.from([1])) && opts[1] === null && opts[2].equals(Buffer.from([0xff, 0xfe])))
  assert.equal(col.dataLen, 3)
  assert.ok(col.dataType.equals(DataType.binary()))
})

test('serie byte codec (including cleared-null canonical identity)', () => {
  const cases = [[Utf8Serie, ['a', null, 'cd', '']], [BinarySerie, [Buffer.from([1]), null, Buffer.from([0xff]), Buffer.alloc(0)]]]
  for (const [cls, values] of cases) {
    const col = new cls(values)
    assert.ok(cls.deserializeBytes(col.serializeBytes()).equals(col))
    col.set(1, values[0]) // clears the last null
    assert.ok(cls.deserializeBytes(col.serializeBytes()).equals(col))
  }
})

test('serie field infers nullability, copy is independent', () => {
  assert.equal(new Utf8Serie(['a', null]).toField('c').nullable, true)
  assert.equal(new Utf8Serie(['a', 'b']).toField('c').nullable, false)
  const original = new Utf8Serie(['a', 'b'])
  const dup = original.copy()
  dup.push('c')
  assert.ok(original.length === 2 && dup.length === 3)
})
