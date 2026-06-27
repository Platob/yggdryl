// Tests for the yggdryl Serie (Arrow-backed column). Build first with `npm run build`,
// then `node --test`.

const { test } = require('node:test')
const assert = require('node:assert')
const { Serie, DataType } = require('..')

test('from values infers type and reads', () => {
  const s = new Serie('n', [1, null, 3])
  assert.strictEqual(s.name, 'n')
  assert.strictEqual(s.numRows, 3)
  assert.strictEqual(s.nullCount, 1)
  assert.strictEqual(s.dataType.toString(), 'int64') // all-integral numbers → int64
  assert.strictEqual(s.category, 'primitive')
  assert.strictEqual(s.get(0), 1)
  assert.strictEqual(s.get(1), null)
  assert.strictEqual(s.get(-1), 3) // negative index
  assert.deepStrictEqual(s.toList(), [1, null, 3])
  assert.ok(s.isNull(1))
  assert.ok(s.isValid(0))
  assert.throws(() => s.get(5))
})

test('infers each scalar kind', () => {
  assert.strictEqual(new Serie('b', [true, false]).dataType.toString(), 'bool')
  assert.strictEqual(new Serie('f', [1.5, 2.5]).dataType.toString(), 'float64')
  assert.strictEqual(new Serie('s', ['a', 'b']).dataType.toString(), 'utf8')
  // a fractional value among integers → float64
  assert.strictEqual(new Serie('m', [1, 2.5]).dataType.toString(), 'float64')
  // binary via the typed factory
  const blob = Serie.binary('raw', [Buffer.from('xy'), null])
  assert.strictEqual(blob.dataType.toString(), 'binary')
  assert.deepStrictEqual(blob.toList()[0], [120, 121]) // 'x','y' bytes
})

test('dtype argument casts', () => {
  const s = new Serie('n', [1, 2, 3], 'int32')
  assert.strictEqual(s.dataType.toString(), 'int32')
  const s2 = new Serie('n', [1, 2], DataType.float(64))
  assert.strictEqual(s2.dataType.toString(), 'float64')
  assert.strictEqual(s2.get(0), 1)
  const nulls = new Serie('n', [null, null], 'int16')
  assert.strictEqual(nulls.dataType.toString(), 'int16')
  assert.strictEqual(nulls.nullCount, 2)
  assert.throws(() => new Serie('n', [null, null]))
})

test('slice, head, resize', () => {
  const s = new Serie('n', [10, 20, 30, 40])
  assert.deepStrictEqual(s.slice(1, 2).toList(), [20, 30])
  assert.deepStrictEqual(s.head(2).toList(), [10, 20])
  assert.deepStrictEqual(s.resize(6).toList(), [10, 20, 30, 40, null, null])
  assert.deepStrictEqual(s.resize(2).toList(), [10, 20])
})

test('cast and categorical', () => {
  const wide = new Serie('n', [1, 2, 3]).cast('float64')
  assert.strictEqual(wide.dataType.toString(), 'float64')
  assert.strictEqual(wide.get(0), 1)
  const cat = new Serie('c', ['a', 'b', 'a', 'a']).categorical()
  assert.strictEqual(cat.isMaterialized, false)
  assert.deepStrictEqual(cat.toList(), ['a', 'b', 'a', 'a'])
  assert.strictEqual(cat.materialize().isMaterialized, true)
})

test('lazy range and index', () => {
  const r = Serie.range(5)
  assert.strictEqual(r.isMaterialized, false)
  assert.deepStrictEqual(r.toList(), [0, 1, 2, 3, 4])
  assert.deepStrictEqual(Serie.range(3, 10, 5).toList(), [10, 15, 20])
  assert.deepStrictEqual(Serie.index(4).toList(), [0, 1, 2, 3])
})

test('nested struct and select', () => {
  const a = new Serie('a', [1, 2])
  const b = new Serie('b', ['x', 'y'])
  const rec = Serie.struct('rec', [a, b])
  assert.strictEqual(rec.category, 'nested')
  assert.strictEqual(rec.children()[0].name, 'a')
  assert.deepStrictEqual(rec.child('b').toList(), ['x', 'y'])
  assert.strictEqual(rec.child(0).name, 'a')
  assert.strictEqual(rec.select('a').get(1), 2)
  assert.strictEqual(rec.select('missing'), null)
  assert.throws(() => rec.select('a.')) // malformed path throws
})

test('display and toString', () => {
  const s = new Serie('n', Array.from({ length: 100 }, (_, i) => i))
  const text = s.display(3)
  assert.ok(text.includes('n: int64'))
  assert.ok(text.includes('97 more rows'))
})

test('bytes and JSON round-trip (including nested)', () => {
  const s = new Serie('n', [1, null, 3])
  assert.deepStrictEqual(Serie.fromBytes(s.toBytes()).toList(), [1, null, 3])
  // toJSON / fromJSON round-trips losslessly through Arrow-IPC
  const json = JSON.stringify(s)
  const back = Serie.fromJSON(JSON.parse(json))
  assert.deepStrictEqual(back.toList(), [1, null, 3])
  const rec = Serie.struct('rec', [new Serie('a', [1, 2])])
  const recBack = Serie.fromBytes(rec.toBytes())
  assert.deepStrictEqual(recBack.select('a').toList(), [1, 2])
})

test('equality', () => {
  const a = new Serie('n', [1, 2, 3])
  const b = new Serie('n', [1, 2, 3])
  const c = new Serie('n', [1, 2, 4])
  assert.ok(a.equals(b))
  assert.ok(!a.equals(c))
})
