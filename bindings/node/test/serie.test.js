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
  // dictionary accessors: distinct values stored once, a code per row
  assert.strictEqual(cat.categoryCount, 2)
  assert.strictEqual(cat.codeAt(0), cat.codeAt(2))
  assert.deepStrictEqual(cat.categories().toList(), ['a', 'b'])
  assert.throws(() => new Serie('n', [1, 2]).categoryCount) // not categorical
})

test('lazy range and index', () => {
  const r = Serie.range(5)
  assert.strictEqual(r.isMaterialized, false)
  assert.deepStrictEqual(r.toList(), [0, 1, 2, 3, 4])
  assert.deepStrictEqual(Serie.range(3, 10, 5).toList(), [10, 15, 20])
  const idx = Serie.index(4)
  assert.deepStrictEqual(idx.toList(), [0, 1, 2, 3])
  // index lookups: label <-> position
  assert.strictEqual(idx.isRange, true)
  assert.strictEqual(idx.at(2), 2)
  assert.strictEqual(idx.position(3), 3)
  assert.strictEqual(idx.contains(3), true)
  assert.strictEqual(idx.contains(4), false)
  // a stepped range is not the canonical 0..len index, but the lookups still work
  const stepped = Serie.range(4, 100, 5)
  assert.strictEqual(stepped.isRange, false)
  assert.strictEqual(stepped.position(110), 2)
  assert.strictEqual(new Serie('n', [1, 2]).isRange, false) // a plain column is not a range
  assert.throws(() => new Serie('n', [1, 2]).at(0)) // not an index
})

test('list factory', () => {
  const nums = Serie.list('nums', [[1, 2], [], null, [3]])
  assert.strictEqual(nums.category, 'nested')
  assert.strictEqual(nums.numRows, 4)
  assert.strictEqual(nums.nullCount, 1)
  assert.strictEqual(nums.valueAt(0), '[1, 2]')
  assert.strictEqual(nums.valueAt(3), '[3]')
  assert.strictEqual(nums.child(0).name, 'item')
  const floats = Serie.list('f', [[1], [2, 3]], 'float64')
  assert.strictEqual(floats.child(0).dataType.toString(), 'float64')
  assert.strictEqual(Serie.fromBytes(nums.toBytes()).valueAt(0), '[1, 2]')
})

test('map factory', () => {
  const m = Serie.map('m', [{ a: 1, b: 2 }, { c: 3 }, null])
  assert.strictEqual(m.category, 'nested')
  assert.strictEqual(m.numRows, 3)
  assert.strictEqual(m.nullCount, 1)
  assert.strictEqual(m.valueAt(0), '{a=1, b=2}')
  assert.strictEqual(m.valueAt(1), '{c=3}')
  assert.strictEqual(Serie.fromBytes(m.toBytes()).valueAt(1), '{c=3}')
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

const { Scalar, Field } = require('..')

function frame() {
  // a 3-row, 2-column frame: id int64, name utf8
  return Serie.struct('df', [new Serie('id', [3, 1, 2]), new Serie('name', ['c', 'a', 'b'])])
}

test('frame shape and projection', () => {
  const df = frame()
  assert.deepStrictEqual(df.shape, [3, 2])
  assert.strictEqual(df.numColumns, 2)
  assert.deepStrictEqual(df.columnNames, ['id', 'name'])
  assert.deepStrictEqual(df.selectColumns(['name']).columnNames, ['name'])
  assert.deepStrictEqual(df.withColumn(new Serie('ok', [true, true, false])).columnNames, ['id', 'name', 'ok'])
  assert.deepStrictEqual(df.dropColumns(['name']).columnNames, ['id'])
  assert.deepStrictEqual(df.rename('id', 'key').columnNames, ['key', 'name'])
})

test('frame rows: filter, sort, stack, toDicts', () => {
  const df = frame()
  assert.deepStrictEqual(df.sortBy('id').toDicts(), [
    { id: 1, name: 'a' },
    { id: 2, name: 'b' },
    { id: 3, name: 'c' },
  ])
  assert.deepStrictEqual(df.filter([true, false, true]).shape, [2, 2])
  assert.deepStrictEqual(df.vstack(df).shape, [6, 2])
  assert.deepStrictEqual(df.withRowIndex('i').columnNames, ['i', 'id', 'name'])
})

test('frame row record and object', () => {
  const df = frame()
  const record = df.row(1)
  assert.deepStrictEqual(record.toObject(), { id: 1, name: 'a' })
})

test('frame selectFields casts and fills', () => {
  const df = frame()
  const target = [
    new Field('name', new DataType('utf8'), true),
    new Field('id', new DataType('int64'), true),
    new Field('score', new DataType('float64'), true),
  ]
  const projected = df.selectFields(target)
  assert.deepStrictEqual(projected.columnNames, ['name', 'id', 'score'])
  assert.strictEqual(projected.child('score').valueAt(0), null) // filled with null
})

test('frame Arrow IPC round-trip', () => {
  const df = frame()
  const back = Serie.fromArrowIpc('df', df.toArrowIpc())
  assert.deepStrictEqual(back.shape, [3, 2])
  assert.deepStrictEqual(back.toDicts(), df.toDicts())
})

test('setAt and push mutate functionally', () => {
  const s = new Serie('n', [1, 2, 3])
  assert.deepStrictEqual(s.setAt(1, new Scalar(20)).toList(), [1, 20, 3])
  assert.deepStrictEqual(s.toList(), [1, 2, 3]) // original untouched
  assert.deepStrictEqual(s.setAt(0, Scalar.null('int64')).toList(), [null, 2, 3])
  assert.deepStrictEqual(s.push(new Scalar(4)).toList(), [1, 2, 3, 4])
  assert.throws(() => s.setAt(9, new Scalar(1)))
})

test('frame ops require a struct column', () => {
  const s = new Serie('n', [1, 2, 3])
  assert.throws(() => s.shape)
})
