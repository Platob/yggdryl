// Tests for the yggdryl Node.js extension's BytesIO.
// Build first with `npm run build`, then run `node --test`.

const { test } = require('node:test')
const assert = require('node:assert')
const { BytesIO } = require('..')

test('read advances the cursor', () => {
  const io = new BytesIO(Buffer.from('hello world'))
  assert.deepStrictEqual(io.read(5), Buffer.from('hello'))
  assert.strictEqual(io.tell(), 5)
  assert.deepStrictEqual(io.read(1), Buffer.from(' '))
  // Omitting size (or a negative one) reads the rest.
  assert.deepStrictEqual(io.read(), Buffer.from('world'))
  assert.deepStrictEqual(io.read(-1), Buffer.from(''))
  assert.strictEqual(io.tell(), 11)
  assert.strictEqual(io.length, 11)
})

test('getValue ignores the cursor', () => {
  const io = new BytesIO(Buffer.from('abcdef'))
  io.read(3)
  assert.deepStrictEqual(io.getValue(), Buffer.from('abcdef'))
  assert.strictEqual(io.tell(), 3)
})

test('stream flag keeps the cursor fixed', () => {
  const io = new BytesIO(Buffer.from('abcdef'), false)
  assert.strictEqual(io.stream, false)
  assert.deepStrictEqual(io.read(3), Buffer.from('abc'))
  assert.deepStrictEqual(io.read(3), Buffer.from('abc'))
  assert.strictEqual(io.tell(), 0)
  io.stream = true
  assert.deepStrictEqual(io.read(3), Buffer.from('abc'))
  assert.strictEqual(io.tell(), 3)
})

test('seek whences and errors', () => {
  const io = new BytesIO(Buffer.from('0123456789'))
  assert.strictEqual(io.seek(4), 4)
  assert.strictEqual(io.seek(2, 1), 6)
  assert.strictEqual(io.seek(-1, 2), 9)
  assert.deepStrictEqual(io.read(), Buffer.from('9'))
  assert.throws(() => io.seek(-1))
  assert.throws(() => io.seek(0, 9))
})

test('write overwrites and zero-fills', () => {
  const io = new BytesIO(Buffer.from('abc'))
  io.seek(1)
  assert.strictEqual(io.write(Buffer.from('XY')), 2)
  assert.deepStrictEqual(io.getValue(), Buffer.from('aXY'))
  io.seek(5)
  io.write(Buffer.from('Z'))
  assert.deepStrictEqual(io.getValue(), Buffer.from([0x61, 0x58, 0x59, 0x00, 0x00, 0x5a]))
})

test('readLine walks lines', () => {
  const io = new BytesIO(Buffer.from('one\ntwo\nthree'))
  assert.deepStrictEqual(io.readLine(), Buffer.from('one\n'))
  assert.deepStrictEqual(io.readLine(), Buffer.from('two\n'))
  assert.deepStrictEqual(io.readLine(), Buffer.from('three'))
  assert.deepStrictEqual(io.readLine(), Buffer.from(''))
})

test('truncate', () => {
  const io = new BytesIO(Buffer.from('abcdef'))
  io.seek(3)
  assert.strictEqual(io.truncate(), 3)
  assert.deepStrictEqual(io.getValue(), Buffer.from('abc'))
  assert.strictEqual(io.truncate(1), 1)
  assert.deepStrictEqual(io.getValue(), Buffer.from('a'))
})
