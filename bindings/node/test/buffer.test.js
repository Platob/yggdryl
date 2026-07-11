'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')

const {
  I8Buffer,
  I32Buffer,
  I64Buffer,
  U8Buffer,
  F32Buffer,
  F64Buffer,
  BooleanBuffer,
} = yggdryl.buffer
const { I64Type } = yggdryl.dtype
const { Whence } = yggdryl.io

test('buffer field() and headers', () => {
  const entries = [{ key: Buffer.from('unit'), value: Buffer.from('ms') }]
  const annotated = new I64Buffer([1, 2, 3]).withHeaders(entries)

  // headers round-trips as an Array<{key, value}>.
  const meta = annotated.headers
  assert.equal(meta.length, 1)
  assert.ok(meta[0].key.equals(Buffer.from('unit')))
  assert.ok(meta[0].value.equals(Buffer.from('ms')))

  // field() hands out the matching typed field, carrying the headers.
  const field = annotated.field('ts', true)
  assert.equal(field.name, 'ts')
  assert.equal(field.nullable, true)
  assert.ok(field.dataType.equals(new I64Type()))
  assert.ok(field.headers[0].value.equals(Buffer.from('ms')))

  // No headers by default; field() defaults nullable to false.
  const plain = new I64Buffer([1, 2, 3])
  assert.equal(plain.headers, null)
  assert.equal(plain.field('ts').nullable, false)
  assert.equal(plain.field('ts').headers, null)

  // The boolean buffer hands out a BooleanField.
  assert.equal(new BooleanBuffer([true, false]).field('flag', true).name, 'flag')

  // Headers is an annotation — it does not change byte identity.
  assert.ok(new I64Buffer([1, 2, 3]).equals(annotated))
})

test('buffer numeric construct + access', () => {
  const buf = new I32Buffer([10, 20, 30])
  assert.equal(Number(buf.length), 3)
  assert.equal(Number(buf.len()), 3)
  assert.ok(!buf.isEmpty())
  assert.equal(buf.get(1), 20)
  assert.equal(buf.get(3), null)
  assert.deepEqual(buf.toArray(), [10, 20, 30])
  assert.ok(new I32Buffer().isEmpty())
})

test('buffer numeric serialize round-trip + validation', () => {
  const buf = new I32Buffer([1, -2, 3])
  const bytes = buf.serializeBytes()
  assert.equal(bytes.length, 12)
  assert.ok(I32Buffer.deserializeBytes(bytes).equals(buf))

  // little-endian layout
  assert.deepEqual(new U8Buffer([1, 2, 3]).asBytes(), Buffer.from([1, 2, 3]))
  assert.deepEqual(
    new I32Buffer([0x01020304]).asBytes(),
    Buffer.from([0x04, 0x03, 0x02, 0x01]),
  )

  // a non-multiple length throws with actionable guidance
  assert.throws(() => I32Buffer.deserializeBytes(Buffer.alloc(6)), /multiple of 4/)
})

test('buffer value semantics', () => {
  const a = new I8Buffer([1, 2, 3])
  const b = new I8Buffer([1, 2, 3])
  assert.ok(a.equals(b))
  assert.equal(a.hashCode(), b.hashCode())
  assert.ok(!a.equals(new I8Buffer([9])))
})

test('buffer i64 round-trips (BigInt-aware)', () => {
  const buf = new I64Buffer([7, 8, 9])
  assert.deepEqual(buf.toArray().map(Number), [7, 8, 9])
  assert.ok(I64Buffer.deserializeBytes(buf.serializeBytes()).equals(buf))

  // bridges to positioned IO
  const cursor = buf.byteCursor()
  assert.deepEqual(cursor.preadI64Array(3, Whence.Start).map(Number), [7, 8, 9])
  assert.ok(I64Buffer.fromByteBuffer(buf.toByteBuffer()).equals(buf))
})

test('buffer float equality is bitwise', () => {
  assert.ok(new F64Buffer([NaN]).equals(new F64Buffer([NaN])))
  assert.ok(!new F64Buffer([0]).equals(new F64Buffer([-0])))

  // f32 marshals over an f64 boundary
  const f = new F32Buffer([1.5, -2.25])
  assert.deepEqual(f.toArray(), [1.5, -2.25])
  assert.ok(F32Buffer.deserializeBytes(f.serializeBytes()).equals(f))
})

test('buffer BooleanBuffer bit-packed', () => {
  const buf = new BooleanBuffer([true, false, true, true, false])
  assert.equal(Number(buf.length), 5)
  assert.equal(buf.get(0), true)
  assert.equal(buf.get(1), false)
  assert.equal(buf.get(5), null)
  assert.equal(Number(buf.countSetBits()), 3)
  assert.deepEqual(buf.toArray(), [true, false, true, true, false])

  // trailing bits canonicalised: 0xFF over 3 bits is only the low three
  const packed = BooleanBuffer.fromBytes(Buffer.from([0xff]), 3)
  assert.equal(Number(packed.countSetBits()), 3)
  assert.ok(packed.equals(new BooleanBuffer([true, true, true])))

  assert.ok(BooleanBuffer.deserializeBytes(buf.serializeBytes()).equals(buf))
  assert.throws(() => BooleanBuffer.fromBytes(Buffer.from([0, 0]), 3))
})

test('buffer namespace surface + Node omissions', () => {
  for (const name of [
    'I8Buffer',
    'I16Buffer',
    'I32Buffer',
    'I64Buffer',
    'U8Buffer',
    'U16Buffer',
    'U32Buffer',
    'F32Buffer',
    'F64Buffer',
    'BooleanBuffer',
  ]) {
    assert.ok(name in yggdryl.buffer, `${name} exported`)
  }
  // U64Buffer is intentionally omitted (no native napi u64 scalar)
  assert.equal(yggdryl.buffer.U64Buffer, undefined)
})
