'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { DataType, I32Buffer, U8Buffer, U256Buffer, F64Buffer } = yggdryl.types

test('the types namespace exposes the typed buffers', () => {
  for (const cls of [I32Buffer, U256Buffer, F64Buffer]) {
    assert.equal(typeof cls, 'function')
  }
})

test('construction and access', () => {
  const b = new I32Buffer([1, 2, 3])
  assert.ok(b.count === 3 && b.length === 3)
  assert.ok(b.get(0) === 1 && b.get(2) === 3)
  assert.equal(b.get(99), null) // out of range -> null
  assert.deepEqual(b.toValues(), [1, 2, 3])
  assert.ok(new I32Buffer().isEmpty())
})

test('mutation', () => {
  const b = new I32Buffer([1, 2, 3])
  b.push(4)
  b.set(1, 20)
  assert.deepEqual(b.toValues(), [1, 20, 3, 4])
  assert.throws(() => b.set(99, 0))
})

test('byte codec', () => {
  const b = new I32Buffer([1, 2, 3])
  assert.ok(I32Buffer.fromBytes(b.toBytes()).equals(b))
  assert.equal(b.toBytes().length, 12) // 3 * 4 bytes
})

test('equality, copy, descriptor', () => {
  const a = new I32Buffer([1, 2, 3])
  assert.ok(a.equals(new I32Buffer([1, 2, 3])) && !a.equals(new I32Buffer([1, 2])))
  const dup = a.copy()
  dup.push(4)
  assert.ok(a.length === 3 && dup.length === 4)
  assert.ok(a.dataType.equals(DataType.i32()))
  assert.equal(a.field('c', false).typeName, 'i32')
})

test('across flavors', () => {
  assert.deepEqual(new U8Buffer([1, 255]).toValues(), [1, 255])
  assert.deepEqual(new F64Buffer([1.5, -2.5]).toValues(), [1.5, -2.5])
  // wide 256-bit values cross as little-endian Buffers
  const w = new U256Buffer([Buffer.alloc(32, 5), Buffer.alloc(32, 9)])
  assert.ok(w.count === 2 && w.get(1).equals(Buffer.alloc(32, 9)))
  assert.ok(U256Buffer.fromBytes(w.toBytes()).equals(w))
})
