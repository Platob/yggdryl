'use strict'

// Phase 8 — vectorized arithmetic (add/sub/mul/div/rem) + reshape (filter / fillNull /
// toList / toStruct / toMap), mirrored from the core `dyn AnySerie` surface. Node has no
// operators, so each is a named camelCase method; `add(other)` folds BOTH paths (a Serie
// operand OR a JS numeric scalar) into one method — the parity of Python's `__add__`/`add`
// (there is no separate `addScalar`). The result of an arithmetic op follows the LEFT operand.

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const {
  StructField,
  StructSerie,
  ListSerie,
  MapSerie,
  I8Serie,
  I32Serie,
  I64Serie,
  I128Serie,
  F64Serie,
  Utf8Serie,
} = yggdryl.types
const { D64Serie, D128Serie } = yggdryl.decimal
const { Date32Serie } = yggdryl.temporal

// A flat numeric struct { x: i32, y: i32 } from two same-length arrays.
function numStruct(xs, ys) {
  const x = new I32Serie(xs)
  const y = new I32Serie(ys)
  const schema = new StructField('s', [x.toField('x'), y.toField('y')], false)
  return StructSerie.fromColumns(schema, [x.serializeBytes(), y.serializeBytes()])
}

// ---------------------------------------------------------------------------------------
// serie × serie — same type and cross type (result follows the LEFT)
// ---------------------------------------------------------------------------------------

test('add of two same-type columns is element-wise', () => {
  const out = new I32Serie([1, 2, 3]).add(new I32Serie([10, 20, 30]))
  assert.ok(out instanceof I32Serie)
  assert.deepEqual(out.toOptions(), [11, 22, 33])
})

test('cross-type add follows the left operand: i32.add(i64) -> i32', () => {
  // The i64 right (its values cross as strings) is cast into the left's i32; result is i32.
  const out = new I32Serie([1, 2]).add(new I64Serie(['10', '20']))
  assert.ok(out instanceof I32Serie)
  assert.deepEqual(out.toOptions(), [11, 22])
})

test('cross-type add follows the left operand: f64.add(i32) -> f64', () => {
  const out = new F64Serie([1.5, 2.5]).add(new I32Serie([1, 2]))
  assert.ok(out instanceof F64Serie)
  assert.deepEqual(out.toOptions(), [2.5, 4.5])
})

test('sub / mul on columns', () => {
  assert.deepEqual(new I32Serie([10, 20]).sub(new I32Serie([1, 2])).toOptions(), [9, 18])
  assert.deepEqual(new I32Serie([2, 3]).mul(new I32Serie([5, 6])).toOptions(), [10, 18])
})

test('a null cell in either operand propagates to the result', () => {
  const out = new I32Serie([1, null, 3]).add(new I32Serie([10, 20, null]))
  assert.deepEqual(out.toOptions(), [11, null, null])
})

// ---------------------------------------------------------------------------------------
// serie × scalar broadcast (one `add` accepts a JS number / numeric string)
// ---------------------------------------------------------------------------------------

test('scalar broadcast: I64Serie add 1 -> every element + 1', () => {
  // A JS `number` broadcasts even into an i64 column (values round-trip as strings).
  const out = new I64Serie(['10', '20', '30']).add(1)
  assert.ok(out instanceof I64Serie)
  assert.deepEqual(out.toOptions(), ['11', '21', '31'])
})

test('scalar broadcast on a float column', () => {
  assert.deepEqual(new F64Serie([1.0, 2.0]).add(0.5).toOptions(), [1.5, 2.5])
})

test('scalar broadcast preserves nulls', () => {
  assert.deepEqual(new I32Serie([1, null, 3]).mul(10).toOptions(), [10, null, 30])
})

// ---------------------------------------------------------------------------------------
// div / rem by zero -> null cell; integer overflow wraps
// ---------------------------------------------------------------------------------------

test('integer division by a zero divisor yields a null cell (no throw)', () => {
  assert.deepEqual(new I32Serie([10, 20]).div(new I32Serie([0, 5])).toOptions(), [null, 4])
})

test('integer remainder by a zero divisor yields a null cell (no throw)', () => {
  assert.deepEqual(new I32Serie([10, 21]).rem(new I32Serie([0, 5])).toOptions(), [null, 1])
})

test('i8 addition overflow wraps (two-complement)', () => {
  // 127 + 1 wraps to -128.
  assert.deepEqual(new I8Serie([127]).add(new I8Serie([1])).toOptions(), [-128])
})

// ---------------------------------------------------------------------------------------
// guided errors: out-of-range right, non-numeric operand, length mismatch
// ---------------------------------------------------------------------------------------

test('a cross-type right operand out of the left range throws a guided error', () => {
  // 300 does not fit i8 (the right is cast into the left's element type, range-checked).
  assert.throws(() => new I8Serie([1]).add(new I64Serie(['300'])), /does not fit|range/i)
})

test('a non-numeric scalar operand throws a guided error', () => {
  assert.throws(() => new I32Serie([1, 2]).add('hello'), /not a valid integer|arithmetic operand|number/i)
})

test('adding a non-numeric column (a utf8 leaf inside a struct) throws the core guided error', () => {
  const a = new I32Serie([1, 2])
  const s = new Utf8Serie(['x', 'y'])
  const schema = new StructField('m', [a.toField('a'), s.toField('s')], false)
  const left = StructSerie.fromColumns(schema, [a.serializeBytes(), s.serializeBytes()])
  const right = StructSerie.fromColumns(schema, [a.serializeBytes(), s.serializeBytes()])
  assert.throws(() => left.add(right), /arithmetic is not supported|utf8/i)
})

test('a length mismatch throws a guided error', () => {
  assert.throws(() => new I32Serie([1, 2]).add(new I32Serie([1])), /different lengths|length/i)
})

// ---------------------------------------------------------------------------------------
// nested struct arithmetic (field-wise), serie and scalar
// ---------------------------------------------------------------------------------------

test('struct add is field-wise and follows the left struct', () => {
  const sum = numStruct([1, 2], [10, 20]).add(numStruct([3, 4], [30, 40]))
  assert.ok(sum instanceof StructSerie)
  assert.equal(sum.length, 2)
  assert.deepEqual(I32Serie.deserializeBytes(sum.columnBytes(0)).toOptions(), [4, 6])
  assert.deepEqual(I32Serie.deserializeBytes(sum.columnBytes(1)).toOptions(), [40, 60])
})

test('struct scalar broadcast reaches every leaf', () => {
  const out = numStruct([1, 2], [10, 20]).add(100)
  assert.deepEqual(I32Serie.deserializeBytes(out.columnBytes(0)).toOptions(), [101, 102])
  assert.deepEqual(I32Serie.deserializeBytes(out.columnBytes(1)).toOptions(), [110, 120])
})

// ---------------------------------------------------------------------------------------
// filter — subset selection + length-mismatch error
// ---------------------------------------------------------------------------------------

test('filter keeps the true rows (values and null-ness)', () => {
  const out = new I32Serie([1, null, 3, 4]).filter([true, true, false, true])
  assert.ok(out instanceof I32Serie)
  assert.deepEqual(out.toOptions(), [1, null, 4])
})

test('filter with a wrong-length mask throws a guided error', () => {
  assert.throws(() => new I32Serie([1, 2, 3]).filter([true, false]), /mask|length/i)
})

test('filter drops whole struct rows', () => {
  const out = numStruct([1, 2, 3], [10, 20, 30]).filter([true, false, true])
  assert.equal(out.length, 2)
  assert.deepEqual(I32Serie.deserializeBytes(out.columnBytes(0)).toOptions(), [1, 3])
})

// ---------------------------------------------------------------------------------------
// fillNull — fills nulls (leaf) + no-op on null + decimal/temporal have no scalar form
// ---------------------------------------------------------------------------------------

test('fillNull replaces nulls with the value', () => {
  const out = new I32Serie([1, null, 3]).fillNull(9)
  assert.ok(out instanceof I32Serie)
  assert.deepEqual(out.toOptions(), [1, 9, 3])
  assert.equal(out.nullCount, 0)
})

test('fillNull with null / undefined is a no-op clone', () => {
  const out = new I32Serie([1, null, 3]).fillNull(null)
  assert.equal(out.nullCount, 1)
  assert.deepEqual(out.toOptions(), [1, null, 3])
})

test('fillNull on a utf8 column fills with a string', () => {
  const out = new Utf8Serie(['a', null, 'c']).fillNull('z')
  assert.deepEqual(out.toOptions(), ['a', 'z', 'c'])
})

test('fillNull on a decimal column with a plain JS value throws (no native scalar form)', () => {
  const col = new D64Serie(4, 2, ['1.50', null])
  assert.throws(() => col.fillNull('2.00'), /not supported|scalar/i)
  // A null fill is still the identity no-op.
  assert.equal(col.fillNull(null).nullCount, 1)
})

test('fillNull on a decimal column via a length-1 Serie carrier fills the null (Python parity)', () => {
  const col = new D128Serie(10, 2, ['1.50', null])
  const carrier = new D128Serie(10, 2, ['9.99']) // same (precision, scale)
  const filled = col.fillNull(carrier)
  assert.ok(filled instanceof D128Serie)
  assert.equal(filled.nullCount, 0)
  assert.deepEqual(filled.toOptions(), ['1.50', '9.99'])
})

test('fillNull carrier with a mismatched scale throws the core guard error', () => {
  const col = new D128Serie(10, 2, ['1.50', null]) // scale 2
  const carrier = new D128Serie(10, 3, ['9.999']) // scale 3 -> mismatch
  assert.throws(() => col.fillNull(carrier), /scale|mismatch|does not|fill/i)
})

test('fillNull on a temporal column via a length-1 Serie carrier fills the null', () => {
  const col = new Date32Serie('d', null, ['2020-01-01', null])
  const carrier = new Date32Serie('d', null, ['2021-06-15']) // same (unit, tz)
  const filled = col.fillNull(carrier)
  assert.ok(filled instanceof Date32Serie)
  assert.equal(filled.nullCount, 0)
  assert.deepEqual(filled.toOptions(), ['2020-01-01', '2021-06-15'])
})

test('fillNull on a temporal column with a plain JS value throws (no native scalar form)', () => {
  const col = new Date32Serie('d', null, ['2020-01-01', null])
  assert.throws(() => col.fillNull('2021-01-01'), /not supported|scalar/i)
})

// ---------------------------------------------------------------------------------------
// toList / toStruct / toMap
// ---------------------------------------------------------------------------------------

test('toStruct wraps a leaf column as a one-field struct (default name "value")', () => {
  const st = new I32Serie([1, 2, 3]).toStruct()
  assert.ok(st instanceof StructSerie)
  assert.equal(st.length, 3)
  assert.equal(st.numColumns, 1)
  assert.equal(st.field(0).name, 'value')
  assert.deepEqual(I32Serie.deserializeBytes(st.columnBytes(0)).toOptions(), [1, 2, 3])
})

test('toStruct accepts an explicit field name', () => {
  const st = new I32Serie([7, 8]).toStruct('n')
  assert.equal(st.field(0).name, 'n')
})

test('toList wraps a leaf column as a list of singletons', () => {
  const list = new I32Serie([1, 2, 3]).toList()
  assert.ok(list instanceof ListSerie)
  assert.equal(list.length, 3)
  assert.deepEqual(list.offsets, [0, 1, 2, 3])
  assert.deepEqual(I32Serie.deserializeBytes(list.itemBytes()).toOptions(), [1, 2, 3])
})

test('toMap on a 2-column struct yields a MapSerie frame', () => {
  const keys = new I32Serie([1, 2])
  const vals = new I32Serie([10, 20])
  const schema = new StructField('kv', [keys.toField('key'), vals.toField('value')], false)
  const st = StructSerie.fromColumns(schema, [keys.serializeBytes(), vals.serializeBytes()])
  const map = MapSerie.deserializeBytes(st.toMap())
  assert.ok(map instanceof MapSerie)
  assert.equal(map.length, 2)
  assert.deepEqual(map.offsets, [0, 1, 2])
  assert.deepEqual(I32Serie.deserializeBytes(map.keys()).toOptions(), [1, 2])
  assert.deepEqual(I32Serie.deserializeBytes(map.values()).toOptions(), [10, 20])
})

test('toMap on a leaf column returns the source frame unchanged (no map coercion)', () => {
  // A leaf has no map reading, so the frame reconstructs as the SOURCE class.
  const back = I32Serie.deserializeBytes(new I32Serie([1, 2]).toMap())
  assert.deepEqual(back.toOptions(), [1, 2])
})

// ---------------------------------------------------------------------------------------
// CROSS-BINDING PARITY — the SAME cases as the Python suite's parity block
// (bindings/python/tests/test_ops.py), asserting the SAME outcome. An arithmetic SCALAR operand
// is coerced to the LEFT column's element type: an integer column requires wholeness (a fractional
// operand is a guided error) and accepts a whole int / integer string; a float column accepts any
// numeric string; a nested column infers an i128 / f64 broadcast; and a real-but-non-castable Serie
// right operand surfaces the core's guided error.
// ---------------------------------------------------------------------------------------

test('parity: integer column rejects a fractional scalar, accepts a whole int / integer string', () => {
  // JS has one numeric type (`2.0 === 2`), so the genuinely fractional 2.5 / "2.5" are the shared
  // error cases (Python additionally rejects the float 2.0, which JS cannot express distinctly).
  assert.throws(() => new I32Serie([1, 2, 3]).add(2.5), /whole number|not a valid integer/i)
  assert.throws(() => new I32Serie([1, 2, 3]).add('2.5'), /not a valid integer/i)
  // A whole int, or an integer-valued numeric string, is accepted (range-checked into the column).
  assert.deepEqual(new I32Serie([1, 2, 3]).add(2).toOptions(), [3, 4, 5])
  assert.deepEqual(new I32Serie([1, 2, 3]).add('5').toOptions(), [6, 7, 8])
  // A whole scalar into a wide i64 column keeps working (i64 crosses as a decimal string).
  assert.deepEqual(new I64Serie(['1']).add(1).toOptions(), ['2'])
})

test('parity: float column accepts a numeric string', () => {
  assert.deepEqual(new F64Serie([1.0]).add('2.5').toOptions(), [3.5])
})

test('parity: nested broadcast marshals a whole int at i128 (fits an i128 leaf)', () => {
  const big = new I128Serie(['1', '2'])
  const schema = new StructField('s', [big.toField('big')], false)
  const st = StructSerie.fromColumns(schema, [big.serializeBytes()])
  const expected = [(10n ** 30n + 1n).toString(), (10n ** 30n + 2n).toString()]

  // A bigint well beyond i64 marshals at i128, so it fits the i128 leaf (i64 would overflow).
  const outBig = st.add(10n ** 30n)
  assert.deepEqual(I128Serie.deserializeBytes(outBig.columnBytes(0)).toOptions(), expected)

  // The integer-string form is accepted identically.
  const outStr = st.add((10n ** 30n).toString())
  assert.deepEqual(I128Serie.deserializeBytes(outStr.columnBytes(0)).toOptions(), expected)

  // A value beyond i128::MAX errors in both bindings.
  assert.throws(() => st.add(10n ** 40n), /128-bit range|does not fit|range/i)
})

test('parity: a non-castable Serie right operand surfaces the core guided error', () => {
  assert.throws(
    () => new I64Serie(['1']).add(new Utf8Serie(['a'])),
    /the right operand must be a numeric column/,
  )
})
