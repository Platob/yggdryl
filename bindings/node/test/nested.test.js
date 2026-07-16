'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const {
  DataType,
  Field,
  StructField,
  StructSerie,
  column,
  columnType,
  I8Serie,
  I16Serie,
  I32Serie,
  I64Serie,
  F64Serie,
  U8Serie,
  U128Serie,
  Utf8Serie,
  BinarySerie,
} = yggdryl.types
const { D64Serie } = yggdryl.decimal

// A flat struct column: { id: i32, name: utf8 (with a null) }.
function table() {
  const ids = new I32Serie([1, 2, 3])
  const names = new Utf8Serie(['ann', null, 'cara'])
  const schema = new StructField('person', [ids.toField('id'), names.toField('name')], false)
  return StructSerie.fromColumns(schema, [ids.serializeBytes(), names.serializeBytes()])
}

// ---- StructField -----------------------------------------------------------------------

test('the types namespace exposes the nested classes', () => {
  for (const cls of [StructField, StructSerie]) {
    assert.equal(typeof cls, 'function')
  }
})

test('struct field shape', () => {
  const schema = new StructField(
    'person',
    [new Field('id', DataType.i64(), false), new Field('name', DataType.utf8(), true)],
    true,
  )
  assert.equal(schema.name, 'person')
  assert.equal(schema.typeName, 'struct')
  assert.ok(schema.nullable)
  assert.equal(schema.numFields, 2)
  assert.equal(schema.indexOf('name'), 1)
  assert.equal(schema.field(1).name, 'name')
  assert.equal(schema.fieldNamed('id').name, 'id')
  assert.equal(schema.fieldNamed('missing'), null)
  assert.deepEqual(schema.fields().map((f) => f.name), ['id', 'name'])
})

test('struct field nests', () => {
  const inner = new StructField('point', [new Field('x', DataType.f64(), false)], false)
  const outer = new StructField('shape', [inner], true)
  assert.equal(outer.numFields, 1)
  const recovered = outer.field(0)
  assert.ok(recovered instanceof StructField)
  assert.equal(recovered.name, 'point')
})

test('struct field builders are immutable', () => {
  const base = new StructField('s', [new Field('a', DataType.i32(), true)], true)
  const renamed = base.withName('t').withNullable(false)
  assert.ok(base.name === 's' && base.nullable)
  assert.ok(renamed.name === 't' && !renamed.nullable)
  const grown = base.withField(new Field('b', DataType.utf8(), true))
  assert.ok(base.numFields === 1 && grown.numFields === 2)
})

test('struct field value semantics', () => {
  const a = new StructField('s', [new Field('a', DataType.i32(), true)], true)
  const b = new StructField('s', [new Field('a', DataType.i32(), true)], true)
  assert.ok(a.equals(b))
  assert.equal(a.hashCode(), b.hashCode())
  assert.ok(StructField.deserializeBytes(a.serializeBytes()).equals(a))
  assert.ok(a.copy().equals(a))
})

// ---- StructSerie -----------------------------------------------------------------------

test('struct serie build and navigate', () => {
  const t = table()
  assert.equal(t.length, 3)
  assert.equal(t.numColumns, 2)
  assert.equal(t.field(1).name, 'name')
  // A child crosses as bytes; reconstruct it with the matching Serie class.
  const idsBack = I32Serie.deserializeBytes(t.columnBytes(0))
  assert.equal(idsBack.get(0), 1)
  const namesBack = Utf8Serie.deserializeBytes(t.columnBytesNamed('name'))
  assert.equal(namesBack.get(0), 'ann')
  assert.equal(namesBack.get(1), null)
  assert.equal(t.columnBytesNamed('missing'), null)
})

test('struct serie schema/column count mismatch throws', () => {
  const schema = new StructField('s', [new Field('a', DataType.i32(), true)], false)
  assert.throws(() => StructSerie.fromColumns(schema, []))
})

test('struct serie serialize round trip', () => {
  const t = table()
  assert.ok(StructSerie.deserializeBytes(t.serializeBytes()).equals(t))
})

test('struct serie nests', () => {
  const x = new I32Serie([1, 2])
  const y = new U8Serie([3, 4])
  const innerSchema = new StructField('p', [x.toField('x'), y.toField('y')], false)
  const inner = StructSerie.fromColumns(innerSchema, [x.serializeBytes(), y.serializeBytes()])

  const tag = new Utf8Serie(['a', 'b'])
  const outerSchema = new StructField('o', [inner.toField('point'), tag.toField('tag')], false)
  const outer = StructSerie.fromColumns(outerSchema, [inner.serializeBytes(), tag.serializeBytes()])

  assert.equal(outer.numColumns, 2)
  assert.ok(outer.field(0) instanceof StructField)
  assert.ok(StructSerie.deserializeBytes(outer.serializeBytes()).equals(outer))
})

test('struct serie value semantics and toString', () => {
  const a = table()
  const b = table()
  assert.ok(a.equals(b))
  assert.ok(a.copy().equals(a))
  assert.ok(a.toString().startsWith('StructSerie(len=3'))
  assert.equal(a.hasNulls, false) // no null struct rows (the name *column* has a null, not a row)
})

test('to_field nullability reflects struct rows, not child nulls', () => {
  const schema = table().toField('person')
  assert.ok(schema instanceof StructField)
  assert.equal(schema.name, 'person')
  assert.equal(schema.nullable, false)
})

// ---- StructSerie deep get/set (the binding-dunder mirror) ------------------------------

test('struct serie getAt reads a deep leaf cell by coordinates', () => {
  const t = table() // { id: i32 [1,2,3], name: utf8 ['ann', null, 'cara'] }
  assert.equal(t.getAt([0, 0]), 1) // field 0 (id), cell 0 -> number
  assert.equal(t.getAt([0, 2]), 3)
  assert.equal(t.getAt([1, 0]), 'ann') // field 1 (name), cell 0 -> string
  assert.equal(t.getAt([1, 1]), null) // a null cell -> JS null
  assert.equal(t.getCell([0, 1]), 2) // getCell is an alias of getAt
})

test('struct serie setAt writes a deep leaf cell then reads it back', () => {
  const t = table()
  t.setAt([0, 1], 99) // id cell 1: 2 -> 99 (JS number cast into the i32 leaf)
  assert.equal(t.getAt([0, 1]), 99)
  t.setAt([1, 0], 'ANN') // name cell 0: 'ann' -> 'ANN'
  assert.equal(t.getAt([1, 0]), 'ANN')
  t.setCell([1, 2], null) // setCell alias; clear the (nullable) name cell 2
  assert.equal(t.getAt([1, 2]), null)
})

test('struct serie getPath resolves a cell (index-terminal) and a column (name-terminal)', () => {
  const t = table()
  assert.equal(t.getPath('id[0]'), 1) // trailing index -> the native cell value
  assert.equal(t.getPath('name[2]'), 'cara')
  // A trailing name addresses a whole sub-column, returned as its serializeBytes() frame.
  const names = Utf8Serie.deserializeBytes(t.getPath('name'))
  assert.equal(names.get(0), 'ann')
  assert.equal(names.get(1), null)
})

test('struct serie getCell / setCell accept coords or an index-terminal path', () => {
  const t = table()
  assert.equal(t.getCell([0, 2]), 3) // coords key
  assert.equal(t.getCell('name[0]'), 'ann') // path key
  t.setCell([0, 0], 7)
  assert.equal(t.getCell([0, 0]), 7)
  t.setCell('name[1]', 'BEE') // sets the (previously null) name cell 1
  assert.equal(t.getCell('name[1]'), 'BEE')
})

test('struct serie setPath writes a cell addressed by an index-terminal path', () => {
  const t = table()
  t.setPath('id[2]', 42)
  assert.equal(t.getPath('id[2]'), 42)
  // A name-terminal path has no in-place scalar set — guided error.
  assert.throws(() => t.setPath('id', 5), /in-place assignment/)
})

test('struct serie setAt writes into a currently-null cell (type read from the column)', () => {
  const t = table() // name row 1 is null
  assert.equal(t.getAt([1, 1]), null)
  t.setAt([1, 1], 'was-null') // the leaf type is read from the name column, not the null cell
  assert.equal(t.getAt([1, 1]), 'was-null')
})

test('struct serie child access mirrors the columnBytes surface', () => {
  const t = table()
  assert.equal(t.numChildren(), 2)
  const ids = I32Serie.deserializeBytes(t.childAt(0))
  assert.equal(ids.get(2), 3)
  const names = Utf8Serie.deserializeBytes(t.childNamed('name'))
  assert.equal(names.get(0), 'ann')
  assert.equal(t.childAt(9), null) // out of range -> null
  assert.equal(t.childNamed('missing'), null)
  // getColumn resolves a sub-column by path.
  const byPath = Utf8Serie.deserializeBytes(t.getColumn('name'))
  assert.equal(byPath.get(2), 'cara')
})

test('struct serie get returns the i-th row as a one-row struct frame', () => {
  const t = table()
  const row1 = StructSerie.deserializeBytes(t.get(1))
  assert.equal(row1.length, 1)
  assert.equal(row1.getAt([0, 0]), 2) // id of row 1
  assert.equal(row1.getAt([1, 0]), null) // name of row 1 (null)
})

test('struct serie deep access surfaces the core guided errors unchanged', () => {
  const t = table()
  assert.throws(() => t.getAt([9, 0]), /out of range/) // field index out of range
  assert.throws(() => t.getAt([0, 99]), /out of bounds/) // cell index past the leaf end
  assert.throws(() => t.get(9), /out of range/)
})

// ---- generic inference factory: yggdryl.types.column / columnType -----------------------
// column() returns the built column's serializeBytes() frame; columnType() names the class.

test('column infers the smallest signed int over the value range', () => {
  assert.equal(columnType([1, 2, 3]).name, 'i8')
  assert.equal(I8Serie.deserializeBytes(column([1, 2, 3])).get(0), 1)
  assert.equal(columnType([300]).name, 'i16') // 300 > i8 max
  assert.equal(I16Serie.deserializeBytes(column([300])).get(0), 300)
  assert.equal(columnType([true, false]).name, 'i8') // a boolean counts as 0 / 1
})

test('column infers f64 when any value is fractional', () => {
  assert.equal(columnType([1.5, 2, 3]).name, 'f64')
  const col = F64Serie.deserializeBytes(column([1.5, 2, 3]))
  assert.equal(col.get(0), 1.5)
  assert.equal(col.get(1), 2)
})

test('column infers utf8 from strings and binary from Buffers', () => {
  assert.equal(columnType(['a', 'b']).name, 'utf8')
  assert.equal(Utf8Serie.deserializeBytes(column(['a', 'b'])).get(0), 'a')

  assert.equal(columnType([Buffer.from([1]), Buffer.from([2, 3])]).name, 'binary')
  const bins = BinarySerie.deserializeBytes(column([Buffer.from([1]), Buffer.from([2, 3])]))
  assert.ok(bins.get(1).equals(Buffer.from([2, 3])))
})

test('column allows nulls (nullable) and defaults empty / all-null to i64', () => {
  const col = I8Serie.deserializeBytes(column([1, null, 3]))
  assert.equal(col.length, 3)
  assert.equal(col.nullCount, 1)
  assert.equal(col.get(1), null)
  assert.equal(columnType([]).name, 'i64') // empty -> i64
  assert.equal(columnType([null, null]).name, 'i64') // all-null -> i64
})

test('column honours an explicit dtype (a DataType or a name string)', () => {
  assert.equal(columnType([1, 2, 3], DataType.i32()).name, 'i32')
  assert.equal(I32Serie.deserializeBytes(column([1, 2, 3], DataType.i32())).get(0), 1)
  assert.equal(columnType([1, 2, 3], 'i64').name, 'i64')
  assert.equal(I64Serie.deserializeBytes(column([1, 2, 3], 'i64')).get(0), '1') // i64 crosses as string
})

test('column raises guided errors on a mix or an unbuildable dtype', () => {
  assert.throws(() => column([1, 'a']), /mix of int, str/) // no shared leaf type
  assert.throws(() => column([1, 2], 'd128'), /needs extra parameters|construct its Serie directly/)
  assert.throws(() => column([1], 'nope'), /unknown data type name/)
})

// ---- regression: confirmed Phase 5b defects (each test would have caught the defect) ----

// FIX 3: a whole-valued number beyond i128 infers f64 (like Python's large float), never an
// "exceed i128" error and never a silent integer clamp.
test('column classifies a whole-valued number beyond i128 as f64 (no overflow error)', () => {
  assert.equal(columnType([1e40]).name, 'f64')
  const col = F64Serie.deserializeBytes(column([1e40]))
  assert.equal(col.length, 1)
  assert.equal(col.get(0), 1e40)
})

// FIX 4: 2**127 == i128::MAX + 1; the strict `< 2^127` guard sends it to f64 (never a silent clamp
// to i128::MAX), while an explicit integer dtype still rejects it with a guided error.
test('column: a number at 2**127 goes to f64, an explicit int dtype rejects it', () => {
  assert.equal(columnType([2 ** 127]).name, 'f64')
  assert.equal(F64Serie.deserializeBytes(column([2 ** 127])).get(0), 2 ** 127)
  assert.throws(() => column([2 ** 127], 'i128'), /whole number|out of range/)
})

// FIX 2: a u128 above i128::MAX (~1.7e38) builds over the full [0, u128::MAX] range.
test('column builds a u128 above i128::MAX (full u128 range)', () => {
  const big = '200000000000000000000000000000000000000' // 2e38, above i128::MAX
  assert.equal(columnType([big], 'u128').name, 'u128')
  const col = U128Serie.deserializeBytes(column([big], 'u128'))
  assert.equal(col.get(0), big) // u128 crosses as a decimal string
})

// FIX 1: a deep setAt into a small-int leaf validates like column()/the flat setter — an
// out-of-range or fractional value throws, never silently ToInt32/ToUint32-wrapping/truncating.
test('struct serie setAt rejects an out-of-range or fractional value into an int leaf', () => {
  const ids = new I32Serie([1, 2, 3])
  const tiny = new U8Serie([1, 2, 3])
  const schema = new StructField('rec', [ids.toField('id'), tiny.toField('tiny')], false)
  const t = StructSerie.fromColumns(schema, [ids.serializeBytes(), tiny.serializeBytes()])

  // 5e9 exceeds i32; must throw, not silently wrap to 705032704 (ECMAScript ToInt32).
  assert.throws(() => t.setAt([0, 0], 5000000000), /out of range for i32/)
  assert.equal(t.getAt([0, 0]), 1) // unchanged

  // A fractional value into any int leaf must throw, not truncate to 3.
  assert.throws(() => t.setAt([1, 0], 3.7), /whole number/)
  assert.equal(t.getAt([1, 0]), 1) // unchanged

  // 300 exceeds u8; must throw.
  assert.throws(() => t.setAt([1, 0], 300), /out of range for u8/)

  // A valid in-range set still works; setCell/setPath share the same validation.
  t.setAt([0, 1], 99)
  assert.equal(t.getAt([0, 1]), 99)
  assert.throws(() => t.setCell([0, 0], 2.5), /whole number/)
  assert.throws(() => t.setPath('id[2]', 9999999999), /out of range for i32/)
})

// FIX 5: a negative / fractional coordinate is a guided error, never a ToUint32 wrap into a
// confusing "out of bounds" or (worse) a silently-wrong cell.
test('struct serie coords reject a negative or fractional coordinate', () => {
  const t = table()
  assert.throws(() => t.getAt([-1, 0]), /non-negative integers/)
  assert.throws(() => t.getAt([0.5, 0]), /non-negative integers/)
  assert.throws(() => t.setAt([0, -1], 5), /non-negative integers/)
  assert.throws(() => t.getCell([0, 1.5]), /non-negative integers/)
})

// FIX 6: slice() returns a fresh sub-range (the Node mirror of Python's nested s[a:b]).
test('struct serie slice returns a sub-range as a fresh struct frame', () => {
  const t = table() // 3 rows: id [1,2,3], name ['ann', null, 'cara']
  const mid = StructSerie.deserializeBytes(t.slice(1, 2))
  assert.equal(mid.length, 2)
  assert.equal(mid.getAt([0, 0]), 2) // id of original row 1
  assert.equal(mid.getAt([1, 1]), 'cara') // name of original row 2
  const tail = StructSerie.deserializeBytes(t.slice(2, 99)) // clamped, never throws
  assert.equal(tail.length, 1)
})

// A decimal LEAF has no native cross-language scalar form, so a deep get/set is a guided error
// pointing to getColumn — parity with the Python binding.
test('struct serie deep get/set of a decimal leaf is a guided error', () => {
  const dec = new D64Serie(10, 2, ['1.00', '2.00'])
  const schema = new StructField('rec', [DataType.d64().field('v', true)], false)
  const struct = StructSerie.fromColumns(schema, [dec.serializeBytes()])
  assert.throws(() => struct.getAt([0, 0]), /not supported through deep indexing|getColumn/)
  assert.throws(() => struct.setAt([0, 0], '3.00'), /not supported|concrete/)
})
