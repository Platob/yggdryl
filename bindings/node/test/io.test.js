'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { Bytes, Whence } = yggdryl.io

test('the io namespace exposes Bytes and Whence', () => {
  assert.equal(typeof Bytes, 'function')
  assert.equal(typeof Whence, 'object')
  // Same integer meaning as POSIX SEEK_SET / SEEK_CUR / SEEK_END.
  assert.equal(Whence.Start, 0)
  assert.equal(Whence.Current, 1)
  assert.equal(Whence.End, 2)
})

test('construct, length, and content', () => {
  assert.equal(new Bytes().length, 0)
  const b = new Bytes(Buffer.from('hello world'))
  assert.equal(b.length, 11)
  assert.equal(b.toBytes().toString(), 'hello world')
  assert.equal(b.position, 0)
})

test('positioned pread / pwrite do not move the cursor', () => {
  const b = new Bytes(Buffer.from('hello world'))
  assert.equal(b.pread(6, 5).toString(), 'world')
  assert.equal(b.pread(6, 100).toString(), 'world') // short near the end
  assert.equal(b.pread(11, 5).length, 0) // at the end
  assert.equal(b.position, 0)

  assert.equal(b.pwrite(6, Buffer.from('earth')), 5)
  assert.equal(b.toBytes().toString(), 'hello earth')

  // Writing past the end grows and zero-fills the gap.
  const b2 = new Bytes(Buffer.from('abc'))
  assert.equal(b2.pwrite(5, Buffer.from('Z')), 1)
  assert.deepEqual([...b2.toBytes()], [97, 98, 99, 0, 0, 90])
})

test('cursor read / write and seek with whence', () => {
  const b = new Bytes()
  assert.equal(b.write(Buffer.from('hello')), 5)
  assert.equal(b.write(Buffer.from(' world')), 6)
  assert.equal(b.position, 11)
  assert.equal(b.toBytes().toString(), 'hello world')

  assert.equal(b.seek(Whence.Start, 6), 6)
  assert.equal(b.read(5).toString(), 'world')
  assert.equal(b.seek(Whence.End, -5), 6)
  assert.equal(b.readToEnd().toString(), 'world')

  b.rewind()
  assert.equal(b.position, 0)
  // seek offset defaults to 0.
  assert.equal(b.seek(Whence.End), 11)
})

test('read_exact and end-of-data errors', () => {
  const b = new Bytes(Buffer.from('hello'))
  assert.equal(b.preadExact(1, 3).toString(), 'ell')
  assert.throws(() => b.preadExact(3, 5), /end of data/) // only 2 remain

  b.seek(Whence.Start, 3)
  assert.throws(() => b.readExact(5))
  assert.equal(b.position, 3) // cursor unchanged on error
  assert.equal(b.readExact(2).toString(), 'lo')
  assert.equal(b.position, 5)
})

test('seek edges: before the start throws, past the end is allowed', () => {
  const b = new Bytes(Buffer.from('hello'))
  assert.throws(() => b.seek(Whence.Start, -1), /before the start/)
  assert.equal(b.seek(Whence.End, 3), 8)
  assert.equal(b.read(4).length, 0) // read past the end is empty
  assert.equal(b.write(Buffer.from('Z')), 1) // write fills the gap
  assert.deepEqual([...b.toBytes()], [104, 101, 108, 108, 111, 0, 0, 0, 90])
})

test('a negative offset or size throws', () => {
  const b = new Bytes(Buffer.from('hello'))
  assert.throws(() => b.pread(-1, 3), /non-negative/)
  assert.throws(() => b.pread(0, -3), /non-negative/)
})

test('slice is zero-copy with copy-on-write', () => {
  const parent = new Bytes(Buffer.from('hello world'))
  const window = parent.slice(6, 5)
  assert.equal(window.toBytes().toString(), 'world')
  assert.equal(window.length, 5)

  // Writing to the slice copies-on-write; the parent is untouched.
  window.pwrite(0, Buffer.from('WORLD'))
  assert.equal(window.toBytes().toString(), 'WORLD')
  assert.equal(parent.toBytes().toString(), 'hello world')

  assert.throws(() => parent.slice(6, 6), /past the end/)
})

test('copy is independent; equality is by content', () => {
  const a = new Bytes(Buffer.from('hello'))
  const dup = a.copy()
  assert.ok(dup.equals(a))
  dup.pwrite(0, Buffer.from('HELLO'))
  assert.equal(dup.toBytes().toString(), 'HELLO')
  assert.equal(a.toBytes().toString(), 'hello') // copy is independent

  // Equality ignores the cursor.
  const other = new Bytes(Buffer.from('hello'))
  other.seek(Whence.Start, 3)
  assert.ok(a.equals(other))
  assert.ok(!a.equals(new Bytes(Buffer.from('world'))))

  assert.equal(a.toString(), 'Bytes(len=5, position=0)')
})
