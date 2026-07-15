'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const {
  DataType,
  I8Scalar,
  I32Scalar,
  I32Serie,
  I256Serie,
  U8Scalar,
  U64Scalar,
  U64Serie,
  U256Scalar,
  F16Scalar,
  F64Scalar,
  F64Serie,
} = yggdryl.types

// A representative scalar per marshaling flavor: [class, present value, its type name].
const SCALARS = [
  [I32Scalar, -5, 'i32'],
  [U8Scalar, 255, 'u8'],
  [U64Scalar, '18446744073709551615', 'u64'],
  [F16Scalar, 1.5, 'f16'],
  [F64Scalar, -2.25, 'f64'],
]

const eq = (a, b) => {
  // Structural equality that also handles Buffer values (u96/u256/…).
  if (Buffer.isBuffer(a) && Buffer.isBuffer(b)) return a.equals(b)
  return a === b
}

// ---------------------------------------------------------------------------------------
// Scalar
// ---------------------------------------------------------------------------------------

test('the types namespace exposes the value classes', () => {
  for (const cls of [I32Scalar, I32Serie, U256Scalar, F64Serie]) {
    assert.equal(typeof cls, 'function')
  }
})

test('scalar present and null', () => {
  for (const [cls, value, name] of SCALARS) {
    const present = new cls(value)
    assert.ok(eq(present.value, value))
    assert.equal(present.isNull, false)
    assert.equal(present.typeName, name)
    assert.ok(present.dataType.equals(DataType.byName(name)))

    for (const nul of [new cls(), new cls(null), cls.null()]) {
      assert.equal(nul.isNull, true)
      assert.equal(nul.value, null)
    }
  }
})

test('scalar equality, hash, and byte codec', () => {
  for (const [cls, value] of SCALARS) {
    const a = new cls(value)
    const b = new cls(value)
    assert.ok(a.equals(b) && a.hashCode() === b.hashCode())
    assert.ok(!a.equals(cls.null()))
    assert.ok(cls.null().equals(cls.null()))
    assert.ok(cls.deserializeBytes(a.serializeBytes()).equals(a))
    assert.ok(cls.deserializeBytes(cls.null().serializeBytes()).equals(cls.null()))
  }
})

test('scalar wide integers cross as a decimal string', () => {
  assert.equal(new U64Scalar('42').value, '42')
  assert.equal(new U256Scalar(Buffer.alloc(32)).value.length, 32)
  assert.throws(() => new U64Scalar('not-a-number'), /u64/)
})

test('scalar 256-bit integers cross as little-endian bytes', () => {
  const buf = Buffer.alloc(32)
  buf.writeUInt32LE(1234, 0)
  assert.equal(new U256Scalar(buf).value.readUInt32LE(0), 1234)
  assert.throws(() => new U256Scalar(Buffer.alloc(8)), /little-endian bytes/)
})

test('scalar small-int range is checked', () => {
  assert.throws(() => new U8Scalar(256), /u8/)
  assert.throws(() => new I8Scalar(200), /i8/)
})

test('scalar field, toSerie, and toString', () => {
  const scalar = new I32Scalar(7)
  const field = scalar.field('x', false)
  assert.ok(field.name === 'x' && field.typeName === 'i32' && field.nullable === false)
  assert.ok(scalar.toSerie().equals(new I32Serie([7])))
  assert.equal(scalar.toString(), 'I32Scalar(7)')
  assert.equal(new I32Scalar().toString(), 'I32Scalar(null)')
})

// ---------------------------------------------------------------------------------------
// Serie
// ---------------------------------------------------------------------------------------

test('serie construction and access', () => {
  const col = new I32Serie([1, null, 3])
  assert.equal(col.length, 3)
  assert.ok(col.nullCount === 1 && col.hasNulls)
  assert.deepEqual(col.toOptions(), [1, null, 3])
  assert.ok(col.get(0) === 1 && col.get(1) === null)
  assert.equal(col.get(99), null) // out of range -> null

  const empty = new I32Serie()
  assert.ok(empty.length === 0 && empty.isEmpty())

  const dense = I32Serie.fromValues([1, 2, 3])
  assert.ok(dense.nullCount === 0 && !dense.hasNulls)
})

test('serie mutation', () => {
  const col = new I32Serie([1, null, 3])
  col.push(4)
  col.push(null)
  assert.deepEqual(col.toOptions(), [1, null, 3, 4, null])
  col.set(1, 20)
  assert.deepEqual(col.toOptions(), [1, 20, 3, 4, null])
  assert.throws(() => col.set(99, 0))
})

test('serie scalar interop', () => {
  const col = new I32Serie([1, null, 3])
  assert.ok(col.getScalar(0).equals(new I32Scalar(1)))
  assert.ok(col.getScalar(1).equals(I32Scalar.null()))
  assert.ok(col.getScalar(99).equals(I32Scalar.null()))
  assert.ok(I32Serie.fromValues([7]).asScalar().equals(new I32Scalar(7)))
  assert.equal(new I32Serie([1, 2]).asScalar(), null)
  assert.ok(I32Serie.fromScalar(new I32Scalar(9)).equals(new I32Serie([9])))
})

test('serie field infers nullability', () => {
  const withNulls = new I32Serie([1, null])
  const dense = new I32Serie([1, 2])
  assert.equal(withNulls.toField('c').nullable, true)
  assert.equal(dense.toField('c').nullable, false)
  assert.equal(dense.field('c', true).nullable, true)
  assert.ok(withNulls.dataType.equals(DataType.i32()))
})

test('serie byte codec round-trips (canonical identity after clearing a null)', () => {
  const col = new I32Serie([1, null, 3])
  col.set(1, 2) // clears the last null; must still round-trip byte-equal
  assert.ok(I32Serie.deserializeBytes(col.serializeBytes()).equals(col))
})

test('serie copy is an independent value', () => {
  const original = new I32Serie([1, 2, 3])
  const dup = original.copy()
  dup.push(4)
  assert.ok(original.length === 3 && dup.length === 4)
})

test('serie round-trips across marshaling flavors', () => {
  const cases = [
    [I32Serie, [1, null, -3]],
    [U64Serie, ['0', null, '18446744073709551615']],
    [F64Serie, [1.5, null, -2.25]],
  ]
  for (const [cls, values] of cases) {
    const col = new cls(values)
    assert.deepEqual(col.toOptions(), values)
    assert.ok(cls.deserializeBytes(col.serializeBytes()).equals(col))
  }

  // Wide-byte flavor: values are little-endian Buffers.
  const wide = new I256Serie([Buffer.alloc(32, 1), null])
  assert.ok(I256Serie.deserializeBytes(wide.serializeBytes()).equals(wide))
  assert.ok(wide.get(0).equals(Buffer.alloc(32, 1)) && wide.get(1) === null)
})
