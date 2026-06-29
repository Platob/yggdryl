'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const { BinaryType, Utf8, Field, Binary, Whence } = require('../index.js')

test('binary type round-trips', () => {
  const b = new BinaryType()
  assert.equal(b.name, 'binary')
  assert.equal(b.toString(), 'binary')
  assert.equal(b.isLarge, false)
  assert.equal(new BinaryType(true).name, 'large_binary')

  assert.ok(BinaryType.fromStr('large_binary').equals(new BinaryType(true)))
  assert.ok(BinaryType.fromMapping(b.toMapping()).equals(b))
  assert.ok(BinaryType.fromBytes(b.toBytes()).equals(b))
  assert.equal(JSON.stringify(b), '"binary"')
})

test('utf8 type aliases', () => {
  const s = new Utf8()
  assert.equal(s.name, 'string')
  assert.ok(Utf8.fromStr('utf8').equals(s))
  assert.ok(Utf8.fromStr('large_utf8').equals(new Utf8(true)))
})

test('field round-trips with metadata', () => {
  const field = new Field('payload', new BinaryType(true), false, { unit: 'bytes' })
  assert.equal(field.name, 'payload')
  assert.ok(field.dataType.equals(new BinaryType(true)))
  assert.equal(field.nullable, false)
  assert.deepEqual(field.metadata, { unit: 'bytes' })

  assert.ok(Field.fromMapping(field.toMapping()).equals(field))
  assert.ok(Field.fromBytes(field.toBytes()).equals(field))
  assert.ok(Field.fromJSON(JSON.parse(JSON.stringify(field))).equals(field))
})

test('binary buffer value and serialization', () => {
  const buf = new Binary(Buffer.from([0, 1, 2]))
  assert.deepEqual([...buf.toBytes()], [0, 1, 2])
  assert.equal(buf.length, 3)
  assert.ok(buf.dataType.equals(new BinaryType()))

  assert.ok(Binary.fromBytes(buf.toBytes()).equals(buf))
  assert.ok(Binary.fromMapping(buf.toMapping()).equals(buf))
  assert.ok(Binary.fromJSON(JSON.parse(JSON.stringify(buf))).equals(buf))

  const large = new Binary(Buffer.from('x'), true)
  assert.ok(large.dataType.equals(new BinaryType(true)))
  assert.ok(Binary.fromMapping(large.toMapping()).equals(large))
})

test('binary implements io', () => {
  const buf = new Binary()
  assert.equal(buf.write(Buffer.from('hello ')), 6)
  assert.equal(buf.write(Buffer.from('world')), 5)
  assert.equal(buf.size, 11)
  assert.ok(buf.capacity >= 11)

  buf.seek(0, Whence.Start)
  assert.equal(buf.read(5).toBytes().toString(), 'hello')
  assert.equal(buf.tell(), 5)
  assert.equal(buf.pread(6, 5).toBytes().toString(), 'world')

  buf.pwrite(0, Buffer.from('HELLO'))
  assert.equal(buf.toBytes().toString(), 'HELLO world')

  buf.resize(5, '.'.charCodeAt(0))
  assert.equal(buf.toBytes().toString(), 'HELLO')

  assert.equal(buf.seek(-1, Whence.End), 4)
  assert.throws(() => buf.seek(-100, Whence.Start))
})
