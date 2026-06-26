// Tests for the yggdryl Node.js extension's BytesIO.
// Build first with `npm run build`, then run `node --test`.

const { test } = require('node:test')
const assert = require('node:assert')
const os = require('node:os')
const path = require('node:path')
const fs = require('node:fs')
const { BytesIO } = require('..')

test('fromStr reads a file else utf8-encodes', () => {
  // A plain string is taken verbatim as UTF-8, via the constructor or fromStr.
  assert.deepStrictEqual(new BytesIO('héllo').getValue(), Buffer.from('héllo', 'utf8'))
  assert.deepStrictEqual(BytesIO.fromStr('héllo').getValue(), Buffer.from('héllo', 'utf8'))
  // Buffers still work unchanged.
  assert.deepStrictEqual(new BytesIO(Buffer.from('raw')).getValue(), Buffer.from('raw'))
  assert.deepStrictEqual(new BytesIO().getValue(), Buffer.from(''))

  // A string naming an existing file is read in as its bytes.
  const p = path.join(os.tmpdir(), `yggdryl_node_${process.pid}_from_str.bin`)
  fs.writeFileSync(p, Buffer.from([0x66, 0x69, 0x6c, 0x65, 0x00, 0x01, 0x02]))
  assert.deepStrictEqual(new BytesIO(p).getValue(), Buffer.from([0x66, 0x69, 0x6c, 0x65, 0x00, 0x01, 0x02]))
  assert.deepStrictEqual(BytesIO.fromStr(p).getValue(), Buffer.from([0x66, 0x69, 0x6c, 0x65, 0x00, 0x01, 0x02]))
  // stream flag still applies on the string path.
  assert.strictEqual(new BytesIO('abc', false).stream, false)
})

test('mode and open', () => {
  const io = new BytesIO(Buffer.from('hello'))
  assert.strictEqual(io.mode, 'r')
  const child = io.open('rb', false)
  assert.strictEqual(child.mode, 'r')
  assert.deepStrictEqual(child.getValue(), Buffer.from('hello'))
  assert.strictEqual(child.stream, false)
  // Write truncates; append (a) positions at the end.
  assert.deepStrictEqual(new BytesIO(Buffer.from('abc')).open('w').getValue(), Buffer.from(''))
  const appender = new BytesIO(Buffer.from('abc')).open('a')
  assert.strictEqual(appender.mode, 'a')
  assert.strictEqual(appender.tell(), 3)
  assert.throws(() => io.open('nope'))
})

test('capacity, reserve and truncate', () => {
  const io = BytesIO.withCapacity(64)
  assert.ok(io.capacity >= 64)
  io.reserveCapacity(128)
  assert.ok(io.capacity >= 128)
  io.write(Buffer.from('abc'))
  // truncate grows (zero-fill) and shrinks.
  assert.strictEqual(io.truncate(5), 5)
  assert.deepStrictEqual(io.getValue(), Buffer.from([0x61, 0x62, 0x63, 0, 0]))
  assert.strictEqual(io.truncate(2), 2)
  assert.deepStrictEqual(io.getValue(), Buffer.from('ab'))
})

test('url, pread and pwrite', () => {
  const io = new BytesIO(Buffer.from('0123456789'))
  assert.strictEqual(io.url.scheme, 'mem')
  io.seek(4)
  // Positional pread/pwrite leave the cursor put (whence omitted = start).
  assert.deepStrictEqual(io.pread(2, 6), Buffer.from('67'))
  assert.strictEqual(io.tell(), 4)
  assert.strictEqual(io.pwrite(Buffer.from('AB'), 0), 2)
  assert.deepStrictEqual(io.getValue().subarray(0, 2), Buffer.from('AB'))
  assert.strictEqual(io.tell(), 4)
  // Cursor-relative (whence=1) uses and advances the cursor.
  assert.deepStrictEqual(io.pread(2, 0, 1), Buffer.from('45'))
  assert.strictEqual(io.tell(), 6)
})

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
