'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const {
  BinaryType,
  Utf8Type,
  Field,
  Binary,
  Utf8,
  Whence,
  JsonFormat,
  setJsonFormat,
  jsonFormat,
  resetJsonFormat,
} = require('../index.js')

test('data types', () => {
  assert.equal(new BinaryType().name, 'binary')
  assert.equal(new Utf8Type(true).name, 'large_string')
  assert.ok(Utf8Type.fromStr('utf8').equals(new Utf8Type()))
  assert.ok(BinaryType.fromBytes(new BinaryType().toBytes()).equals(new BinaryType()))
  assert.equal(JSON.stringify(new Utf8Type()), '"string"')
})

test('field with string type', () => {
  const field = new Field('name', new Utf8Type(), false, { k: 'v' })
  assert.ok(field.dataType.equals(new Utf8Type()))
  assert.ok(Field.fromJSON(JSON.parse(JSON.stringify(field))).equals(field))
})

test('binary value and io', () => {
  const buf = new Binary(Buffer.from([0, 1, 2]))
  assert.deepEqual([...buf.toBytes()], [0, 1, 2])
  assert.equal(buf.length, 3)
  assert.ok(buf.dataType.equals(new BinaryType()))
  assert.ok(Binary.fromBytes(buf.toBytes()).equals(buf))

  const io = new Binary()
  io.write(Buffer.from('hello '))
  io.write(Buffer.from('world'))
  io.seek(0, Whence.Start)
  assert.equal(io.read(5).toBytes().toString(), 'hello')
  assert.equal(io.pread(6, 5).toBytes().toString(), 'world')
})

test('utf8 value', () => {
  const s = new Utf8('héllo')
  assert.equal(s.value, 'héllo')
  assert.equal(s.toString(), 'héllo')
  assert.ok(s.dataType.equals(new Utf8Type()))
  assert.ok(Utf8.fromBytes(s.toBytes()).equals(s))
  assert.ok(Utf8.fromJSON(JSON.parse(JSON.stringify(s))).equals(s))
})

test('cast and set data type', () => {
  const buf = new Binary(Buffer.from('hi'))

  const text = buf.cast(new Utf8Type())
  assert.ok(text instanceof Utf8)
  assert.equal(text.value, 'hi')
  assert.ok(text.cast(new BinaryType()).equals(buf))

  assert.throws(() => new Binary(Buffer.from([0xff, 0xfe])).cast(new Utf8Type()))

  buf.setDataType(new BinaryType(true))
  assert.ok(buf.dataType.equals(new BinaryType(true)))
  assert.throws(() => new Binary(Buffer.from('hi')).setDataType(new Utf8Type()))
})

test('global json format', () => {
  const field = new Field('c', new BinaryType(), true)
  assert.ok(!field.toJsonString().includes('\n'))
  try {
    setJsonFormat(new JsonFormat(true, 2))
    assert.ok(jsonFormat().isPretty)
    assert.ok(field.toJsonString().includes('\n'))
  } finally {
    resetJsonFormat()
  }
  assert.ok(!field.toJsonString().includes('\n'))
})
